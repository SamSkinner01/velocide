# Lobby and match flow

Hosted and joined games gather in a **lobby** before each match: a player list with pings
and ready state, mode / level / host settings, ready-up, chat. The host starts the match, the
level loads on everyone, and at the end everyone returns to the lobby. Single-player never
sees the lobby.

| where | what |
|---|---|
| `shared::lobby` | `GameMode`, `MatchSettings` (+ clamp / validate), `LevelInfo` + `scan_levels`, `LobbyPhase`, `LobbyPlayer`, `LobbyState`, messages `LobbyRequest` / `LobbyUpdate` |
| `server::lobby` | `LobbyConfig`, `LobbyCommand`, the `MatchFlow` state and every match-flow method of `Server` |
| `client::net::lobby` | `NetClient` side: lobby state, level catalog, pings, host commands |
| `client::menus::lobby` | `LobbyScreen` (the UI) and `Handoff` (a session moving between screens) |
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
| `Lobby` | players connected, **no bodies**, no ticks (the world is kept but not simulated; the tick counter stands still) | lobby screen |
| `Countdown { secs }` | settings locked, 3 → 1 broadcast as it changes | lobby screen with a countdown band |
| `InMatch` | fresh `SimWorld` for `settings.level`, every player and bot spawned, `LoadLevel` to all | game screen |
| `Results` | the level keeps running (so the world stays visible and connections healthy) | game screen with the results panel |

`LobbyPhase::level_running()` is true for `InMatch` and `Results`. Joining during those goes
straight in (a body at the next spawn point + `LoadLevel`); joining during `Lobby` /
`Countdown` lands in the lobby (and is spawned when the match starts).

**Match start** (`Server::start_match`): load the level from disk (validated again), build a
new `SimWorld` (doors, encounters, everything level-side starts fresh), **carry the tick
counter over** (client snapshot ordering and clocks stay monotonic), store the settings in
`world.match_settings`, spawn every player in id order then bots, reset lag compensation,
per-client delta baselines and queued inputs (`InputQueue::discard_pending` keeps the sequence
position), send `LoadLevel`, then the `InMatch` state.

**Return to lobby** (`Server::return_to_lobby`): a fresh, empty `SimWorld` of the same level
(so nothing from the match lingers), bodies gone, everyone un-readied (bots stay ready),
`Lobby` state broadcast. Clients unload their level copy and prediction when they see the
lobby phase. If every human leaves during a match the server returns to the lobby by itself.

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

struct LevelInfo { name, title, kind: LevelKind }   // scan_levels(): assets/levels/*.ron + built-in
enum LobbyPhase { Lobby, Countdown { secs: u8 }, InMatch, Results }
struct LobbyPlayer { id, name, ready, color: u8, ping_ms: u16, is_host, bot }
struct LobbyState { phase, settings, players, host: Option<PlayerId>, lobby_enabled, autostart,
                    results_timeout: Option<u16> }   // all_ready(), ready_count(), ping(id), player(id)
```

**Level rules**: campaign needs a `LevelKind::Campaign` level, deathmatch an `Arena`;
`ANY_MODE_LEVELS` (`proving_ground`, `movement_test`) work in both. The server validates
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
| `ServerMessage::Lobby(LobbyUpdate::State(LobbyState))` | S→C | all | on every change (checked at 30 Hz) and once per second (fresh pings) |
| `…::Levels(Vec<LevelInfo>)` | S→C | joining client | once on connect |
| `…::Notice(String)` | S→C | the sender | why a request was refused ("Only the host can do that", "… is a campaign level: deathmatch needs an arena", "Not everyone is ready (FORCE START starts anyway)") |

Validation: lobby requests and ready toggles share a per-player token bucket (burst 10, then
4/s; the rest is dropped silently, so a spamming client can't flood everyone with state
broadcasts). Everything else (decode limits, per-update message caps) is the existing protocol
hardening (`docs/network.md`).

On connect the order is `Welcome`, [`LoadLevel` if a level runs], `Tuning`, `PlayerList`,
`Lobby(Levels)`, then the `Lobby(State)` broadcast. At match start `LoadLevel` always precedes
the `InMatch` state.

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
velocide-server [--level cistern_nine] [--mode campaign|deathmatch] [--max-players 8]
                [--frag-limit 20] [--time-limit 600] [--friendly-fire] [--autostart] [--no-lobby]
```

Without `--mode` the level's kind decides. An incompatible `--mode` / `--level` pair is an
error at startup. `--max-players` is clamped to 8 (4 for co-op).

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
 MainMenu ─HOST / JOIN─▶ LobbyScreen ─(LevelLoaded / InMatch state)─▶ InGame
    ▲  ▲                    ▲                                           │
    │  └── LEAVE / errors ──┤◀──── Transition::ReturnToLobby (Handoff) ─┘
    └─────── SINGLE PLAYER ─┴──────────────────────────────▶ InGame (no lobby)
```

- `Transition::Start(Host | Join)` creates a `LobbyScreen` (`LobbyScreen::start` makes the
  session). It shows a connecting panel until the first lobby state, then the lobby.
- When a level arrives (`NetEvent::LevelLoaded`, or a running-phase state) the lobby returns
  `Transition::EnterMatch`; the app moves the session (`LobbyScreen::into_handoff` → `Handoff
  { session, target, chat }`) into `InGame::from_handoff`, which builds the already received
  level. Both screens take events **one at a time** and stop at the hand-over, so the game
  screen still sees the `Spawned` that follows.
- When the server goes back to the lobby the game screen returns
  `Transition::ReturnToLobby`; the app moves the session back (`InGame::into_handoff` →
  `LobbyScreen::from_handoff`). Chat history travels along.
- Joining a match in progress goes through the lobby screen in one frame. A server without a
  lobby (old protocol semantics) also works: no lobby state but in game ⇒ `EnterMatch`.
- `InGame::start(Join)` straight into a lobby server also works (it hands over on the first
  `Lobby` state).

### Lobby screen layout

Title + `HOSTING ON PORT 47420` (or the server address); top right: who you are, **LOBBY**,
and for hosts `FRIENDS JOIN WITH 192.168.1.20:47420 (UDP)` (`net::lan::local_lan_ip`: a UDP
socket `connect`ed to 8.8.8.8 — no packet is sent — falling back to private ranges, then to a
generic hint). Left: **PLAYERS n/max** (colour swatch, host crown, name, ping coloured by
`hud::ping_color`, READY / ... / BOT, open slots; the host gets a mouse-only `X` kick column)
and **CHAT** (log + field, Enter or SEND; `T` focuses it). Right: **MATCH** — for the host
MODE / LEVEL (levels of the mode first, sandboxes marked TEST) / MAX PLAYERS / FRAG LIMIT +
TIME LIMIT (deathmatch, presets incl. NONE) / FRIENDLY FIRE (co-op); everyone else sees the
same as read-only rows. Then READY (default focus), START MATCH (enabled when all are ready) +
FORCE START or CANCEL START (host), LEAVE LOBBY, and a status line. A countdown band (number
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
  shown. Gameplay input stops, the cursor is released, Esc doesn't open the pause menu.
- `NetEvent::Notice` lines appear in the chat log in red.

### Command line

```text
velocide --host [PORT] [--level L] [--mode campaign|deathmatch] [--max-players N] [--autostart]
velocide --connect IP:PORT [--ready]
         --ready       ready up automatically 1 s after entering the lobby (--bot implies it)
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
  No ticks (so no combat, AI or FX) run in the lobby; dev chat commands answer "no match is
  running" there and `spawn_enemies` returns nothing.
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
cargo test -p client --test lobby          # 11 headless tests (virtual clock, real UDP)
cargo test -p client --test lobby_screen   # real lobby + game screens on a headless GPU
cargo run -p client --release --example lobby_shots -- out/   # PNGs: host / guest lobby,
        # countdown, match, scoreboard with pings, host pause menu, results, lobby after
scripts/local_test.sh --release            # dedicated server (--autostart) + 2 clients (--ready)
scripts/local_test.sh --lobby              # ... staying in the lobby (P1 hosts, starts by hand)
```

`client/tests/lobby.rs` covers: host-only settings / start (guest requests refused with a
notice), clamping, START needs all ready, 3 s countdown → the chosen level on every client
with exact prediction; invalid level/mode pairs and too many players for co-op; settings
locked outside the lobby; join-in-progress; dedicated host migration (+ listen servers never
migrate, wrong tokens ignored); return to lobby (fresh level: door state reset, ready reset,
second match with working inputs); results phase + timeout; autostart (cancelled by a
not-ready joiner); ping propagation (≈120 ms RTT player shows 90–250 ms everywhere, also in
match); disconnects during countdown and match (empty server returns to the lobby); server
full + raising the limit; ready spam rate limit; no-lobby servers.

## Known limitations

- No lobby browser / server list: join by IP:port (the session layer's `JoinTarget` is where
  a Steam lobby / matchmaker would plug in, see `docs/network.md`).
- Kick is mouse-only (a small `X` per row); there is no ban list.
- The level catalog is scanned once at server start; levels added later need a restart.
- The countdown is fixed at 3 s and can't be configured per server.
- Chat history isn't sent to players who join later.
- The first lobby state on a listen server can show no host for one round trip.
- Player colours follow the slot (`PlayerId`); `LobbyPlayer::color` is there for a future
  colour picker.
