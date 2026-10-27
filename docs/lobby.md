# Lobby and match flow

Hosted and joined games gather in a **lobby** before each match: a player list with pings,
ready state and everyone's look, mode / level / host settings, ready-up, chat. While they wait
the players run around the **warm-up** hub, the Vestry (a movement course, a firing range, a
practice pit, a jump-pad tower and a ready board); the lobby is an overlay over it. The host
starts the match, the level loads on everyone, and at the end everyone returns to the warm-up.
Single-player never sees the lobby.

| where | what |
|---|---|
| `shared::lobby` | `GameMode`, `MatchSettings` (+ clamp / validate), `LevelInfo` + `scan_levels`, `LobbyPhase`, `LobbyPlayer`, `LobbyState`, messages `LobbyRequest` / `LobbyUpdate` |
| `server::lobby` | `LobbyConfig`, `LobbyCommand`, the `MatchFlow` state and every match-flow method of `Server` (incl. the warm-up world and cosmetics) |
| `shared::sim::warmup` | the warm-up rules on a hub world: respawns, nothing scored, PvP by the next mode, practice consoles and range dummies |
| `shared::cosmetics` | `Cosmetics` (mask, crest, cloak, emblem, colours, flame) and the catalogues |
| `client::net::lobby` | `NetClient` side: lobby state, level catalog, pings, host commands, our look |
| `client::menus::lobby` | `LobbyPanel` (the lobby UI and its state, used by both), `LobbyScreen` (menu-style, no world) and `Handoff` (a session moving between screens) |
| `client::ingame::warmup` | the warm-up on the game screen: the lobby overlay, the HUD card, the countdown banner, the hub's ready board and console signs |
| `client::menus::customize` | the CUSTOMISE screen (`CustomizeMenu`, turning preview), `LookSync`, look swatches |
| `client::ingame::match_flow` | in-game side: phase changes, results screen, scoreboard rows |
| `client::ingame::scoreboard` | scoreboard table with pluggable score columns |

## Phases

```text
                 host START (all ready) / FORCE START / autostart
   ┌────────┐ ─────────────────────────────────────────────▶ ┌─────────────┐
   │ Lobby  │ ◀──────────────── host CANCEL ──────────────── │ Countdown 3 │
   └────────┘                                                └──────┬──────┘
     ▲    ▲                                                         │ 3 s: start_match()
     │    │  host RETURN TO LOBBY (pause menu)                      ▼
     │    └──────────────────────────────────────────────── ┌─────────────┐
     │                                                       │   InMatch   │
     │   host CONTINUE / 30 s timeout   ┌─────────┐ ◀─────── └─────────────┘
     └──────────────────────────────────│ Results │  Server::end_match() (mode rules)
                                        └─────────┘
```

| phase | server | clients |
|---|---|---|
| `Lobby` | the **warm-up**: a `SimWorld` on the hub ([`WARMUP_LEVEL`] `vestry`) with every player and bot in it, ticking (see below). Without a warm-up: **no bodies**, no ticks (the world is kept but not simulated; the tick counter stands still) | game screen with the lobby overlay (without a warm-up: the lobby screen) |
| `Countdown { secs }` | settings locked, 3 → 1 broadcast as it changes; the warm-up keeps running | the same, with a countdown banner |
| `InMatch` | fresh `SimWorld` for `settings.level`, every player and bot spawned, `LoadLevel` to all | game screen |
| `Results` | the level keeps running (so the world stays visible and connections healthy) | game screen with the results panel |

`LobbyPhase::level_running()` is true for `InMatch` and `Results`; `LobbyState::warmup` says the
server runs the warm-up in `Lobby` / `Countdown`, and `LobbyState::world_running()` is either.
Joining while a world runs goes straight in (a body at the next spawn point + `LoadLevel`);
joining a lobby without a world lands in the lobby screen (and is spawned when the match
starts).

### The warm-up

Every lobby server runs it unless started with `--no-warmup` (`LobbyConfig::warmup`; also off
when the hub file is missing or isn't `LevelKind::Hub`: then the lobby has no world, as
before). The hub is loaded once at startup and a **fresh** warm-up world is built whenever the
server enters its lobby: at startup, and on every return from a match (`Server::enter_warmup`:
`replace_world` with the hub, `SimWorld::start_warmup(next settings)`, every member spawned in
id order, `LoadLevel` to all — then the caller broadcasts the `Lobby` state). The match world
replaces it at match start as before (`load_match_world`).

Rules (`shared::sim::warmup`, hooked into the death / respawn rule and `step_modes`):

- Deaths respawn after `WARMUP_RESPAWN_SECS` (1 s) at the hub spawn farthest from everyone; no
  downed state, no checkpoint reset.
- **Nothing is scored**: no stats, scoreboard, limits, `Ended` or results; the style meter still
  runs (practice).
- Players hurt each other only if the **selected next mode** is PvP (`MatchSettings::mode.is_pvp()`,
  re-applied whenever the host changes the settings: `SimWorld::set_warmup_next`); otherwise
  their hits only knock back. Practice Moulds get no co-op scaling.
- The warm-up runs the **balance profile of the selected mode** (`Server::update_effective_tuning`
  uses the lobby's mode while the world is the warm-up; re-run on every settings change, so the
  clients get the matching `Tuning`). `SimWorld::warmup` is set too: loadout changes apply at
  once (docs/arsenal.md); `SimWorld::in_warmup()` = the warm-up rules are on. Highlights and
  play-of-the-game recording only run during a match.
- **Practice stations** read from the hub's encounters (`step_encounters` only runs campaign
  levels): an encounter whose trigger has no volume is a **fixture** — its first wave stands from
  the start (the firing range's six **Effigies**, docs/enemies.md: they show every hit, never
  attack or move, can't die and heal to full 3 s after the last hit; names the registry doesn't
  know are skipped) and a dummy that does die is back 3 s later. Any other encounter is a
  **console**: a living player *pressing* `USE` inside its trigger summons its next wave
  (cycling; the previous pack is removed) into its `arena` (the pen) at base health
  (`SpawnOptions { aware: true, .. }`: no difficulty scaling, no elites, they know where the
  players are); a Mould more than 2 m out of the pen is put back where it was summoned; each summon reports
  `SimEvent::WaveStarted { index, wave, waves, enemies }` (clients name the pack).
- Dev chat commands (host) work in the warm-up; `spawn_enemies` works there.

The hub's props: the ready board (`models/prop_ready_board.gltf`) and the console
(`models/prop_console.gltf`), found by the client by model name (`ingame::warmup::HubView`).

**Match start** (`Server::start_match`): load the level from disk (validated again), build a
new `SimWorld` (doors, encounters, everything level-side starts fresh), **carry the tick
counter over** (client snapshot ordering and clocks stay monotonic), store the settings in
`world.match_settings`, spawn every player in id order then bots, reset lag compensation,
per-client delta baselines and queued inputs (`InputQueue::discard_pending` keeps the sequence
position), send `LoadLevel`, then the `InMatch` state.

**Return to lobby** (`Server::return_to_lobby`): a fresh warm-up (above) — or without one a
fresh, empty `SimWorld` of the same level with every body gone — everyone un-readied (bots stay
ready), `Lobby` state broadcast. Clients keep their world when the state says `warmup` (the hub
arrived just before with `LoadLevel`), and unload their level copy and prediction otherwise. If
every human leaves during a match the server returns to the lobby by itself.

**No-lobby servers** (`LobbyConfig::enabled == false`: single-player, tests, `--no-lobby`)
start in `InMatch` and still broadcast the state (pings, `lobby_enabled: false`); for them
`return_to_lobby()` restarts the level.

## Shared model (`shared::lobby`)

```rust
enum GameMode { Campaign, Deathmatch }   // title(), id(), parse("dm"), max_players() 4 / 8,
                                         // accepts(&LevelInfo), has_score_limits(), has_friendly_fire()
struct MatchSettings {
    mode: GameMode,
    level: String,            // file stem / built-in name
    max_players: u8,          // 1..=8, co-op 1..=4
    frag_limit: u32,          // deathmatch, default 20, 0 = none, max 200
    time_limit_secs: u32,     // deathmatch, default 600, 0 = none, max 3600
    friendly_fire: bool,      // co-op, default off
}
settings.clamp();                         // numbers into range, level name cleaned (never fails)
settings.validate(&levels)?;              // level exists and suits the mode -> player-facing error
settings.fit_to(&levels);                 // clamp + pick a level of the mode (UI after a mode change)
settings.summary(&levels);                // "DEATHMATCH // CISTERN NINE // 20 FRAGS // 10 MIN"

struct LevelInfo { name, title, kind: LevelKind }   // scan_levels(): assets/levels/*.ron + built-in; is_hub()
enum LobbyPhase { Lobby, Countdown { secs: u8 }, InMatch, Results }
struct LobbyPlayer { id, name, ready, color: u8, ping_ms: u16, is_host, bot, cosmetics: Cosmetics }
struct LobbyState { phase, settings, players, host: Option<PlayerId>, lobby_enabled, autostart,
                    results_timeout: Option<u16>, warmup: bool }
// all_ready(), ready_count(), ping(id), player(id), world_running(), in_warmup()
```

**Level rules**: campaign needs a `LevelKind::Campaign` level, deathmatch an `Arena`;
`ANY_MODE_LEVELS` (`proving_ground`, `movement_test`) work in both. A `LevelKind::Hub` level (the
Vestry) is never a match level: `validate` refuses it in every mode ("… is the warm-up hall, not
a match level"), `fit_to` never picks it, and the host / single-player pickers don't list it. The server validates
against **its own** catalog, which it sends to clients on connect, so the lobby offers levels
a joining client doesn't have installed (`LoadLevel` carries the whole level anyway).

## Messages

All on the reliable channel, inside one new variant each (`PROTOCOL_VERSION` 3):

| message | direction | who | notes |
|---|---|---|---|
| `ClientMessage::SetReady { ready }` | C→S | anyone | (existing message) now rate-limited; also re-broadcasts `PlayerUpdated` |
| `ClientMessage::Lobby(LobbyRequest::ClaimHost { token })` | C→S | listen-server host | sent automatically after `Welcome` when `ClientConfig::host_token` is set |
| `…::SetSettings(MatchSettings)` | C→S | host | Lobby phase only; clamped, validated; the player limit must fit everyone present |
| `…::StartMatch { force }` | C→S | host | starts the countdown; without `force` everyone must be ready |
| `…::CancelCountdown` | C→S | host | |
| `…::ReturnToLobby` | C→S | host | during `InMatch` / `Results` |
| `…::Kick { player }` | C→S | host | not yourself; the kicked client sees "kicked by the host" |
| `ClientMessage::SetCosmetics(Cosmetics)` | C→S | anyone | sent right after `Welcome` (`ClientConfig::cosmetics`) and whenever the look changes (the client debounces: at most every 0.5 s); any phase; sanitised (`Cosmetics::sanitized`), rate-limited with the lobby requests, re-broadcast as `PlayerUpdated` |
| `ServerMessage::Lobby(LobbyUpdate::State(LobbyState))` | S→C | all | on every change (checked at 30 Hz) and once per second (fresh pings) |
| `…::Levels(Vec<LevelInfo>)` | S→C | joining client | once on connect |
| `…::Notice(String)` | S→C | the sender | why a request was refused ("Only the host can do that", "… is a campaign level: deathmatch needs an arena", "Not everyone is ready (FORCE START starts anyway)") |

Validation: lobby requests, ready toggles and look changes share a per-player token bucket
(burst 10, then 4/s; the rest is dropped silently, so a spamming client can't flood everyone
with state broadcasts).

**Looks** travel in `PlayerInfo::cosmetics` (player list, joins, updates) and
`LobbyPlayer::cosmetics`; new players start with the default look until their
`SetCosmetics` arrives; server bots get a varied look of their own (`server::lobby::bot_cosmetics`). Everything else (decode limits, per-update message caps) is the existing protocol
hardening (`docs/network.md`).

On connect the order is `Welcome`, [`LoadLevel` if a world runs: the match or the warm-up],
`Tuning`, `PlayerList`, `Lobby(Levels)`, then the `Lobby(State)` broadcast. At match start
`LoadLevel` always precedes the `InMatch` state, and on the way back the hub's `LoadLevel`
precedes the `Lobby` state.

## Host rules

- **Listen server**: `Session::start` gives the in-process server a random
  `LobbyConfig::host_token` (OS randomness, `session::random_host_token`) and the local client
  presents it (`ClaimHost`). Only that player is host; the role never passes to a guest (their
  server is in the host's process anyway). A wrong token is ignored (logged). The first lobby
  state can arrive one round trip before the claim (host shown as nobody for a frame). Dev
  chat commands are the host's only.
- **Dedicated server** (no token): the longest-connected human is host; when they leave, the
  next one in join order becomes host and the server says "*NAME is now the host*". Bots never
  host. An empty server has no host until someone joins. Dev chat commands are off unless the
  server was started with `--dev-commands` (then for the host only).
- **Autostart** (`LobbyConfig::autostart`, `velocide-server --autostart`, client
  `--autostart` when hosting): whenever every connected human is ready the server starts the
  countdown itself; if someone stops being ready or a not-ready player joins, it is cancelled.
  For dedicated servers without anyone at the keyboard. On every lobby server the results
  screen also returns by itself after `RESULTS_TIMEOUT_SECS` (30 s), with or without
  autostart (single-player waits for the player).
- `ServerCommand::Lobby(LobbyCommand)` / `Server::set_match_settings`, `begin_countdown`,
  `start_match`, `end_match`, `return_to_lobby` control the flow without host checks (tools,
  tests, a future admin console).
- **Server full**: the transport gets `max(max_players, 8) + 2` netcode slots, so a full
  lobby answers `Rejected { "server is full" }` (shown as "Couldn't connect: server is full")
  and the host can raise the limit in the lobby. Version mismatches are rejected on connect
  with both protocol versions in the reason.

## Server API

```rust
ServerConfig { lobby: LobbyConfig { enabled, autostart, host_token, settings }, .. }
ServerConfig::host(port, level, max_players)        // lobby enabled (listen servers)
ServerConfig::default() / single_player(level)       // no lobby

server.lobby_phase() -> LobbyPhase
server.world_running() / in_warmup() -> bool         // a world runs / it's the warm-up
server.match_settings() -> &MatchSettings
server.lobby_host() -> Option<PlayerId>
server.levels() -> &[LevelInfo]
server.lobby_state() -> LobbyState
server.set_match_settings(s) -> Result<(), String>
server.begin_countdown(force) -> Result<(), String>
server.start_match() -> Result<(), String>          // immediately (load level, spawn everyone)
server.end_match()                                   // InMatch -> Results
server.return_to_lobby()                             // -> Lobby (no lobby: restart the level)
server.handle_lobby_command(LobbyCommand)            // = ServerCommand::Lobby(..)
```

`ServerConfig::level` / `max_players` are the initial level and limit; `LobbyConfig::settings`
supplies mode, limits and friendly fire (its `level` / `max_players` are overwritten from the
config). If the mode doesn't suit the level, the level's natural mode is used.

Dedicated server (`velocide-server`) starts in the lobby:

```text
velocide-server [--level cistern_nine] [--mode campaign|deathmatch|gungame|grind]
                [--difficulty easy|normal|hard|brutal|inferno] [--max-players 8]
                [--frag-limit 20] [--time-limit 600] [--friendly-fire] [--autostart] [--no-lobby]
                [--no-warmup]
```

Without `--mode` the level's kind decides. An incompatible `--mode` / `--level` pair is an
error at startup. `--max-players` is clamped to 8 (4 for co-op and grind). `--difficulty`
applies to the PvE modes (campaign, grind). `--no-warmup` keeps the lobby without a world.

## Pings

Once per second the server reads renet's RTT for every connection and puts it in
`LobbyPlayer::ping_ms` (bots: 0, shown as "-"). The state broadcast carries them in every
phase, so the lobby list and the in-game scoreboard show everyone's ping. Clients use the
server's number for others and their own RTT for themselves (`NetClient::player_ping`), so
your own ping matches the HUD. Values are quantised to the 60 Hz flush interval (a loopback
player shows ~11–17 ms).

## Client

### Screens

```text
 MainMenu ─HOST / JOIN─▶ LobbyScreen ─(LevelLoaded: the hub or a match)─▶ InGame ◀─┐
    ▲  ▲                    ▲   (connecting; lobby without a world)       │  warm-up ⇄ match
    │  └── LEAVE / errors ──┤◀── Transition::ReturnToLobby (Handoff) ─────┤  in the same screen
    │                            (only when the lobby has no world)        │
    └─────── SINGLE PLAYER ─┴────────────────────────────────▶ InGame (no lobby)
```

- `Transition::Start(Host | Join)` creates a `LobbyScreen` (`LobbyScreen::start` makes the
  session). It shows a connecting panel until the first lobby state, then the lobby.
- When a level arrives (`NetEvent::LevelLoaded` — with a warm-up that's the hub, right after
  `Welcome` — or a running-phase state) the lobby returns `Transition::EnterMatch`; the app moves
  the session (`LobbyScreen::into_handoff` → `Handoff { session, target, chat }`) into
  `InGame::from_handoff`, which builds the already received level. Both screens take events
  **one at a time** and stop at the hand-over, so the game screen still sees the `Spawned` that
  follows.
- With a warm-up the game screen never leaves: the match's `LoadLevel` replaces the hub, the
  hub's `LoadLevel` replaces the match level after the results, and the lobby is the overlay.
- When the server goes back to a lobby **without** a world the game screen returns
  `Transition::ReturnToLobby`; the app moves the session back (`InGame::into_handoff` →
  `LobbyScreen::from_handoff`). Chat history travels along.
- Joining a match in progress goes through the lobby screen in one frame. A server without a
  lobby (old protocol semantics) also works: no lobby state but in game ⇒ `EnterMatch`.
- `InGame::start(Join)` straight into a lobby server also works (it hands over on the first
  `Lobby` state).

### The warm-up on the game screen (`ingame::warmup`)

- **Lobby overlay**: the same `LobbyPanel` as the lobby screen (`PanelMode::Overlay`) over the
  dimmed world: players (identity swatch, look swatch, crown, name, ping, status), chat, the
  match panel (host settings or read-only rows, READY, START / FORCE / CANCEL, CUSTOMISE +
  LOADOUT, WARM UP + LEAVE LOBBY). It opens **by itself on the first arrival** in a session (not for `--bot`
  clients); TAB / Select (the scoreboard binding: there is no scoreboard in the warm-up), Esc or
  WARM UP close it, TAB opens it again. While it is open you stand still (neutral inputs), the
  cursor is free and the HUD, name tags and chat log are hidden. With the overlay closed, Esc
  opens the pause menu as usual (its RETURN TO LOBBY is hidden in the warm-up).
- **Ready key**: `Action::Ready` (F1 / d-pad down) toggles ready from the world or the overlay
  (and on the lobby screen), with a click.
- **HUD card** (top left, overlay closed): WARM-UP, `NEXT: DEATHMATCH // CISTERN NINE`, one dot
  per player in their identity colour (filled = ready) with `3/5 READY`, and your state with the
  keys (`F1 READY UP   TAB LOBBY` / `READY (F1 CANCEL)`). No level timer.
- **Countdown**: the MATCH STARTING band with the number and the summary, on top of the world and
  of the overlay.
- **The hub**: the ready board on the nave's screen wall lists the next match, every player in
  their identity colour with `>` and READY / ... / BOT, and `3/5 READY` / `MATCH STARTS IN 3`
  (world-space text meshes on the board prop, rebuilt only when the text changes); the practice
  console's sign reads `PRACTICE PIT` / `NEXT: 2 CENSERS + 2 GNAWERS`; standing in its USE zone
  shows `E SUMMON …` under the crosshair; a summon shows the banner `PRACTICE: 4 GNAWERS` with
  the wave sound.
- **CUSTOMISE** (from the overlay): the customise sub-screen with the preview in front of the
  world (`docs/models.md` "Cosmetics"). **LOADOUT** (from the overlay, or the pause menu): the
  arsenal's loadout screen (`menus::loadout`, docs/ui.md), its weapon turning in front of the
  world the same way; the client sends the loadout each change and the warm-up world
  (`SimWorld::warmup`) applies it at once. While a sub-screen is
  open the overlay and Ready keys are off.

### Lobby screen layout

(Shown while connecting and on servers whose lobby has no world; the overlay has the same
panels under a slim WARM-UP LOBBY header.)

Title + `HOSTING ON PORT 47420` (or the server address); top right: who you are, **LOBBY**,
and for hosts `FRIENDS JOIN WITH 192.168.1.20:47420 (UDP)` (`net::lan::local_lan_ip`: a UDP
socket `connect`ed to 8.8.8.8 — no packet is sent — falling back to private ranges, then to a
generic hint). Left: **PLAYERS n/max** (identity colour swatch — the marker shape with shape
cues — then a look swatch: coat colour, mask colour, flame dot; host crown, name, ping coloured
by `hud::ping_color`, READY / ... / BOT, open slots; the host gets a mouse-only `X` kick column)
and **CHAT** (log + field, Enter or SEND; `T` focuses it). Right: **MATCH** — for the host
MODE / LEVEL (levels of the mode first, sandboxes marked TEST) / MAX PLAYERS / FRAG LIMIT +
TIME LIMIT (deathmatch, presets incl. NONE) / FRIENDLY FIRE (co-op); everyone else sees the
same as read-only rows (`host_settings` / `settings_rows`: new settings go in both). Then READY
(default focus), START MATCH (enabled when all are ready) + FORCE START or CANCEL START (host),
the player's own buttons (`player_buttons`: CUSTOMISE + LOADOUT, sub-screens of the panel:
`LobbyPanel::customize_ui` / `loadout_ui`), LEAVE LOBBY,
and a status line. A countdown band (number
pops in, settings summary) covers the lower screen. Esc while typing leaves the chat field;
otherwise it asks "LEAVE LOBBY?" (hosts are warned everyone gets disconnected). Server notices
appear bottom-left for 6 s.

The host edits a local draft: changes are sent at most every 0.15 s; a sent edit stays on
screen until the server's state adopts it (or 1 s passes, i.e. it was refused and the notice
explains why). Changing the mode re-fits the level and player limit (`MatchSettings::fit_to`).

### In game

- **Scoreboard** (Tab): everyone with crown, ping and status plus per-mode score columns
  (`scoreboard::columns_for`: deathmatch FRAGS / DEATHS / STYLE, campaign KILLS / STYLE) whose
  cells are `-` placeholders until the mode / style code fills them in
  `InGame::score_rows`.
- **Pause menu**: the multiplayer host gets RETURN TO LOBBY.
- **Results** (`LobbyPhase::Results`): a MATCH OVER banner, the scoreboard with the match
  summary, and CONTINUE TO LOBBY / LEAVE GAME (host), "WAITING FOR THE HOST" + LEAVE GAME
  (others), RESTART / MAIN MENU (single-player). The countdown to the automatic return is
  shown. Gameplay input stops, the cursor is released, Esc doesn't open the pause menu. Every
  row shows the player's look swatch (`ResultsScreen::looks`).
- `NetEvent::Notice` lines appear in the chat log in red.

### Command line

```text
velocide --host [PORT] [--level L] [--mode campaign|deathmatch|gungame|grind]
         [--difficulty easy|normal|hard|brutal|inferno] [--max-players N] [--autostart]
velocide --connect IP:PORT [--ready]
         --ready       ready up automatically 1 s after entering the lobby / the warm-up (--bot implies it)
         --autostart   hosting: the server starts once everyone is ready (implies --ready)
```

The host menu page has MODE, LEVEL (filtered by mode), PORT and MAX PLAYERS; the last mode is
saved (`Settings::host_mode`).

## How game-mode rules plug in

- **Read the rules** from `world.match_settings` (`mode`, `frag_limit`, `time_limit_secs`,
  `friendly_fire`) inside `SimWorld::step` / your systems. It is set before any player spawns,
  by `SimWorld::apply_match_settings`, which also sets the combat `rules` (deathmatch → PvP;
  co-op → enemy scaling, friendly fire from the lobby toggle).
- **Fresh world per match**: `Server::replace_world` carries server configuration over (the
  enemy registry loaded from `assets/enemies/`, the tick counter), purges pending FX, resets lag
  compensation, delta baselines and queued inputs. Per-world dev toggles (`/god`, `/ff`) reset.
  In a lobby **without** a world no ticks (so no combat, AI or FX) run; dev chat commands
  answer "no match is running" there and `spawn_enemies` returns nothing. The warm-up is a
  world with its own rules (above): game-mode code must not treat it as a match
  (`SimWorld::in_warmup()`; `world.match_settings` is the hub's, Campaign; the next match's
  settings are the lobby's).
- **Match clock**: a new `SimWorld` is built for every match (`SimWorld::new` → your mode
  state starts fresh), but `world.tick` continues from the previous world. Count elapsed match
  time in your own state (e.g. ticks stepped since creation), not from `tick == 0`.
- **Ending a match**: emit a `SimEvent` (e.g. `MatchOver { winner }`) from the sim, and in
  `Server::run_tick`'s event loop call `self.end_match()` (after broadcasting whatever results
  data clients need as `GameEvent`s). For single-player campaigns the same applies; the
  results panel offers RESTART / MAIN MENU there.
- **Results screen**: extend `InGame::results_ui` (`ingame/match_flow.rs`) with grades, times,
  per-player stats (store what your `GameEvent`s delivered on `InGame` / a helper struct).
- **Scoreboard**: fill `ScoreRow::cells` (and sort rows) in `InGame::score_rows`; change the
  columns in `scoreboard::columns_for`.
- **Join-in-progress / respawn**: joiners get `world.add_player` at the next spawn point while
  `InMatch`; deathmatch spawn-point choice, respawn timers and checkpoints belong to the mode
  code (`add_player` / `restart_level` are the hooks).
- **Ready / player count**: `LobbyState` is server-side truth; the sim doesn't need it.

## Tests and tools

```text
cargo test -p client --test lobby          # 14 headless tests (virtual clock, real UDP)
cargo test -p client --test lobby_screen   # real lobby + game screens on a headless GPU
cargo test -p server --test warmup_net     # warm-up phases, rules, cosmetics over UDP
cargo test -p shared --lib warmup          # practice stations, respawns, PvP by next mode
cargo run -p client --release --example lobby_shots -- out/   # PNGs: the warm-up overlay (host
        # / guest), the hub with the HUD card and the ready board, customise and loadout (+ its
        # weapon picker) over the world, the firing range's Effigies, a practice pack, the
        # countdown banner, match, scoreboard with pings, host pause menu, results, back in the
        # warm-up, the lobby screen without a world + its loadout and leave dialog
scripts/local_test.sh --release            # dedicated server (--autostart) + 2 clients (--ready)
scripts/local_test.sh --lobby              # ... staying in the lobby (P1 hosts, starts by hand)
```

`client/tests/lobby.rs` covers: host-only settings / start (guest requests refused with a
notice), clamping, everyone in the warm-up with a body, START needs all ready, 3 s countdown
(the warm-up running through it) → the chosen level on every client with exact prediction;
invalid level/mode pairs and too many players for co-op; settings locked outside the lobby;
join-in-progress; dedicated host migration (+ listen servers never migrate, wrong tokens
ignored); the warm-up after every match (prediction exact in the hub, ready reset, a fresh
match level) and the same return to a lobby without a world (no bodies, door state reset);
results phase + timeout; autostart (cancelled by a not-ready joiner); ping propagation
(≈120 ms RTT player shows 90–250 ms everywhere, also in match); disconnects during countdown
and match (empty server returns to the lobby); server full + raising the limit; ready spam rate
limit; no-lobby servers; dev commands in the warm-up and refused in an empty lobby.
`client/tests/lobby_screen.rs` drives the real screens: the hub hands the session to the game
screen at once, autostart → match → join-in-progress → the host's RETURN TO LOBBY brings both
back to the warm-up without a screen change → second match; the overlay (opens on arrival,
TAB / Esc / Select), the Ready key without the overlay, CUSTOMISE (the look reaches the server,
DONE saves); and the old lobby screen round trip on a server without a warm-up.
`server/tests/warmup_net.rs`: the warm-up around every match (bodies, bots, the hub refused as a
match level, PvP only with a PvP next mode, ~1 s respawns, no scoreboard / results, the countdown
keeps ticking, late joiners, the empty server returns to its warm-up), `--no-warmup`, and looks
(shared with the welcome, sanitised, rate-limited, bots' own looks).

## Known limitations

- No lobby browser / server list: join by IP:port (the session layer's `JoinTarget` is where
  a Steam lobby / matchmaker would plug in, see `docs/network.md`).
- Kick is mouse-only (a small `X` per row); there is no ban list.
- The level catalog is scanned once at server start; levels added later need a restart.
- The countdown is fixed at 3 s and can't be configured per server.
- Chat history isn't sent to players who join later.
- The first lobby state on a listen server can show no host for one round trip.
- Player identity colours follow the slot (`PlayerId`); `LobbyPlayer::color` is there for a
  future colour picker. (Cosmetic colours are separate: they never replace the identity colour.)
- A practice console remembers its next pack per world; a client that joins later shows pack 1
  as next on the console's sign until someone summons.
- The overlay opens by itself only on the first arrival in a session, not after every match.
