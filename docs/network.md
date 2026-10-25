# VELOCIDE networking

Authoritative server, client-side prediction with reconciliation, entity interpolation,
delta-compressed snapshots and lag compensation. Single-player runs the same server in-process
over loopback UDP, so SP, co-op and PvP share one code path.

```
 client (game)                                             server (authoritative)
 ─────────────                                             ──────────────────────
 fixed tick:  PlayerInput{seq, view_tick} ──unreliable──▶  InputQueue (dedupe, de-jitter,
   predict (shared movement)   (+5 older unacked inputs)       1 input = 1 movement step)
   keep input history                                      SimWorld::step  (inputs → movement
                                                              → future systems → world rules)
 frame:  reconcile ◀── own MoveState + last seq ──┐        LagCompHistory.record(hitboxes)
         interpolate remote ◀── entity delta ─────┴─unrel─  every 2nd tick: snapshot per client
         events ◀────────── reliable-ordered ───────────── joins/leaves, chat, level, tuning…
```

## Where things live

| module | contents |
|---|---|
| `shared::protocol` | constants, message enums, encode/decode with limits, validation (`sanitize_*`), `ConnectRequest` |
| `shared::protocol::snapshot` | `NetId`, `NetEntity` (quantised), `EntityState` (float), `NetFrame`, `encode_delta` / `decode_delta`, entity `kind`s |
| `shared::sim` | `SimWorld`, `SimPlayer`, `PlayerId`, `TickInputs`, `SimEvent`, `player_move_step`, hitboxes, `HitboxHistory`, built-in level |
| `server` | `Server`, `ServerConfig`, `ServerHandle`, `ServerStats`, `ServerCommand` |
| `server::input_queue` | per-player input queue (dedupe, de-jitter buffer, catch-up, anti speed-hack) |
| `server::lagcomp` | `LagCompHistory` (per-tick hitbox ring, rewind) |
| `server::netsim` | `NetSimConfig`, `NetSim` (latency/jitter/loss/duplication) |
| `server::transport` | renet channel config, `UdpLink`, `ServerTransport` trait, `NetcodeServerTransport` |
| `client::net` | `NetClient`, `NetEvent`, `NetStats`, `ConnectionState`, `ClientConfig` |
| `client::net::{prediction, interpolation, clock, transport, session}` | `Prediction`, `SnapshotBuffer`, `ClockSync`, `ClientTransport`, `Session` |

## Transport

- **renet 2** for channels/reliability over **netcode** (`renetcode`) on UDP, `Unsecure`
  authentication (join by IP:port). `PROTOCOL_ID = 0x56454C4F43494445` ("VELOCIDE").
- The client's `ConnectRequest { protocol_version, name }` travels in netcode's 256-byte
  connect-token user data. The server checks `PROTOCOL_VERSION` on connect and answers a
  mismatch with `ServerMessage::Rejected { reason }`, disconnecting 0.5 s later (so the reason
  arrives). Full servers are refused by netcode itself ("server denied connection").
- We use `renetcode::NetcodeServer/NetcodeClient` directly with ~100 lines of glue (the same
  as `renet_netcode`'s transports) because `renet_netcode` owns its socket and offers no hook
  for the network simulator. All datagrams pass through a `UdpLink` (byte counters + netsim).
- renet is flushed **at most once per tick** on both sides: every flush also emits an ack
  packet, so flushing on every (sub-millisecond) host-loop iteration would flood the peer.

### Channels (`server::connection_config`)

| id | type | client → server | server → client |
|---|---|---|---|
| 0 `CHANNEL_RELIABLE` | reliable-ordered, resend 250 ms | `ClientMessage` (512 KB budget) | `ServerMessage` (32 MB budget; levels can be large) |
| 1 `CHANNEL_UNRELIABLE` | unreliable | `ClientUnreliable` (inputs) | `ServerUnreliable` (snapshots) |

Messages are serde enums encoded with bincode 2 (`config::standard()`: varint integers,
fixed-size floats). Decoding untrusted bytes uses `decode_client_message` (16 KB limit) /
`decode_server_message` (16 MB limit), rejects trailing bytes, and never panics or
over-allocates (bincode's limit is checked before allocating containers).

### Messages

| message | channel | notes |
|---|---|---|
| `ClientMessage::Chat { text }` | C→S reliable | rate-limited (token bucket: burst 5, then 1/s), sanitised, >1 KB dropped |
| `ClientMessage::SetReady { ready }` | C→S reliable | lobby ready-up (rate-limited) |
| `ClientMessage::Lobby(LobbyRequest)` | C→S reliable | host commands: claim host, settings, start, cancel, return to lobby, kick (docs/lobby.md) |
| `ClientUnreliable::Input(InputPacket { ack_tick, inputs })` | C→S unreliable | every client tick; newest input + up to 5 older unacked ones; `ack_tick` = newest snapshot received |
| `ServerMessage::Welcome(Welcome)` | S→C reliable | player id, tick rate, snapshot rate, server tick, version |
| `ServerMessage::Rejected { reason }` | S→C reliable | then disconnect |
| `ServerMessage::LoadLevel { name, level }` | S→C reliable | the full `LevelDef` so prediction collides against exactly the server's geometry (and clients can join servers with levels they don't have) |
| `ServerMessage::Tuning(Tuning)` | S→C reliable | on connect and on every change |
| `PlayerList`, `PlayerJoined`, `PlayerLeft`, `PlayerUpdated` | S→C reliable | lobby / scoreboard state |
| `ServerMessage::Chat { from, text }` | S→C reliable | `from: None` = server |
| `ServerMessage::Game(GameEvent)` | S→C reliable | gameplay events that must arrive (now: `Respawned`; combat adds kills, damage, parries, style…) |
| `ServerMessage::Lobby(LobbyUpdate)` | S→C reliable | lobby state (phase, settings, players with ping / ready / host) on change and at 1 Hz; level catalog on connect; notices (docs/lobby.md) |
| `ServerUnreliable::Snapshot(SnapshotMessage)` | S→C unreliable | see below |

Connection flow: netcode handshake → server `ClientConnected` → validate user data → assign
`PlayerId` (lowest free, ≤ the lobby's player limit) → if a match is running, spawn in
`SimWorld` → send `Welcome`, `LoadLevel` (only while a level runs), `Tuning`, `PlayerList`,
`Lobby(Levels)`; broadcast `PlayerJoined` and the lobby state → snapshots start (while a level
runs). The client is `Loading` until it has the welcome, the level and a snapshot containing
its own state, then `InGame`. Servers with a lobby (listen and dedicated) start without a
level: clients wait in the lobby until the match starts (`docs/lobby.md`).

## Ticks and rates

- Simulation: fixed **60 Hz** (`ServerConfig::tick_rate`), `dt = shared::sim::tick_dt(rate)` on
  both sides (identical float expression ⇒ identical prediction).
- Snapshots: **30 Hz** by default (`tuning.net.snapshot_rate`, hot-reloadable; or
  `ServerConfig::snapshot_rate` / `--snapshot-rate` to pin it) ⇒ every 2nd tick. Clients
  measure the interval from received snapshot ticks (smallest recent gap), so a changed rate is
  picked up automatically.
- Client inputs: one packet per client tick (60 Hz).

## Inputs

`PlayerInput` (`shared::input`) = one tick: `seq`, movement axes (i8), absolute yaw/pitch,
buttons, weapon slot/mode, and `view_tick`/`view_frac` (interpolation time the client was
displaying, for lag compensation).

**Each input is exactly one movement step** on the client and the server, which makes
prediction exact. Validation (`protocol::sanitize_input`, idempotent, applied by the client
*before* predicting and by the server on receipt): axes clamped to ±127, non-finite angles → 0,
pitch clamped to ±89°, yaw clamped to ±1e6 (not wrapped: that would change the value the client
predicted with), unknown button bits cleared, invalid weapon slot/mode → 0. At most 16 inputs
per packet are looked at (the newest).

**Redundancy:** every packet carries the newest input and up to `INPUT_REDUNDANCY - 1 = 5`
older unacknowledged ones, so a burst of up to 5 lost packets loses nothing. Acknowledgement is
implicit: snapshots carry the recipient's `last_input_seq`.

**Server input queue** (`server::input_queue`, per player):
- keeps inputs with `last_seq < seq ≤ last_seq + 1024`, deduplicated by seq;
- hands out **one input per tick** in seq order, after a small adaptive **de-jitter buffer**
  (target = ⌈2 × measured arrival jitter⌉ ticks, max 4; jitter is RFC 3550-style from
  `arrival_tick − seq`), so bursty arrival doesn't make the player stutter for others;
- **catches up** with 2 per tick when more than target+2 are queued (4 above target+8), but
  only while a **token bucket** (refill 1.01/tick, capacity 8) allows it — a client sending
  inputs faster than real time (speed hack) gets ~1 input per tick, the excess piles up and is
  dropped beyond 64 queued (`inputs_overflow`);
- a missing seq (loss burst longer than the redundancy) is skipped and counted
  (`inputs_skipped`); reconciliation fixes the client;
- when the queue runs dry the player simply doesn't move that tick (`input_stalls`), then
  re-buffers.

## Snapshots

`SnapshotMessage` (per client):

| field | meaning |
|---|---|
| `tick` | server tick described (state after simulating it) |
| `baseline_tick` | tick of the frame `entities` is a delta against; `None` = full |
| `last_input_seq` | recipient's last processed input |
| `own: Option<OwnState>` | recipient's own **unquantised** `MoveState` after `last_input_seq` (exact reconciliation). Combat adds health, hard damage, cooldowns, style here |
| `entities` | delta-encoded entity block |

**Entities.** Everything replicated is a generic `NetEntity` keyed by `NetId`:
`kind: u16`, `owner: u16`, `pos` (i32×3, 1/256 m ≈ 4 mm), `vel` (i32×3, 1/64 m/s),
`yaw: u16` (2π/65536), `pitch: i16` (same scale), `flags: u32`, `state: u16`, `health: u16`,
`aux: u32`. Meaning of `flags/state/health/aux/owner` is per `kind`. Id ranges: players
`0..256` (= `PlayerId`), dynamic entities from 1024 (allocated by `SimWorld::alloc_net_id`,
never reused), level-static entities from 2²⁴ (doors: `2²⁴ + door id`). Kinds today:
`PLAYER` (flags `LOW_PROFILE`, `INVULNERABLE`), `DOOR` (`aux` = door id, flag `OPEN`; clients
apply door states to their collision world so prediction collides like the server).

**Delta encoding** (`encode_delta(baseline, current)`), byte-oriented with LEB128 varints:

```
varint changed_count
  per changed or new entity (ascending id):
    varint id gap (first: id; then id - prev - 1)
    varint field mask: POS VEL YAW PITCH FLAGS STATE HEALTH AUX | KIND OWNER
    POS/VEL: 3 × zigzag varint delta to baseline   YAW: zigzag wrapping i16 delta
    PITCH: zigzag delta                             others: absolute varints
varint removed_count
  per removed entity: varint id gap
```

Unchanged entities and fields cost nothing; a moving player costs ~9–15 bytes; an unchanged
frame is 2 bytes. New entities are encoded against `NetEntity::default()`. A full snapshot is
a delta against the empty frame. The decoder validates everything (varint length, ascending
ids, value ranges, entity count ≤ 8192, no trailing bytes).

**Baselines.** The client acknowledges the newest snapshot tick it decoded in every input
packet (`ack_tick`; an ack-only packet is sent when no inputs flow). The server keeps the last
64 frames it sent each client (`Arc<NetFrame>`, shared between clients) and encodes against the
acknowledged one; if it is unknown or older than 120 ticks (2 s) it sends a full snapshot. The
client keeps decoded frames for 180 ticks (> the server's limit), so a baseline is always
available unless the client was cut off for seconds (then it gets a full snapshot).

## Client: prediction and reconciliation (`client::net::prediction`)

1. `NetClient::tick(input)`: assign `seq`, `view_tick/frac`, sanitise, run
   `shared::sim::player_move_step` (the function the server runs) and push
   `(input, predicted state after it)` into the history (≤ 512).
2. On a newer snapshot with own state: drop acknowledged inputs; compare the authoritative
   `MoveState` with the predicted state for `last_input_seq` (**bit-exact** comparison — the
   movement is deterministic and the state travels unquantised). Equal ⇒ nothing to do.
3. Otherwise (explosion knockback, teleport, lost input, door timing): reset to the server state
   and **replay** the unacknowledged inputs; count the correction (`corrections`, error at the
   acked input, `max_shift` of the current position).
4. **Smoothing:** the shift of the current predicted position goes into a `render_offset`
   (and the previous-tick position is shifted too), so the drawn position doesn't move at the
   moment of correction. The offset decays as `offset *= exp(-correction_smoothing · dt)`
   (12/s ⇒ ~83 ms time constant) per frame; if it is larger than `correction_snap_distance`
   (3 m) we snap instead. `render_position(alpha)` = lerp(previous tick, current tick, alpha) +
   offset.

## Client: clock sync, interpolation, view time (`clock`, `interpolation`)

- **Clock sync:** for each snapshot, `sample = snapshot_tick − local_ticks_at_arrival`. A
  smoothed offset (EMA 0.05; resync if a sample is > 30 ticks off) gives `latest_tick()` (the
  newest snapshot tick we should have now); `server_tick()` adds RTT/2 (renet's RTT). The mean
  absolute deviation of the samples is the **jitter**.
- **Interpolation delay** = snapshot interval + `interp_delay_ms` (50) +
  `interp_jitter_factor` (2) × jitter, capped at `interp_delay_max_ms` (250) and adapted with a
  ~0.5 s time constant. At 30 Hz with ±20 ms network jitter this settles around 100–105 ms.
- **Render clock:** `render_tick` follows `latest_tick() − delay`; it advances with real time
  and is **time-stretched** (±10 %) towards the target rather than jumping (snaps only if more
  than 30 ticks off), so remote motion stays smooth while the estimate adapts.
- **Interpolation:** remote entities (everything except the local player) are sampled at
  `render_tick` between the two surrounding frames: positions/velocity lerped, yaw shortest-arc,
  pitch lerped, discrete fields from the nearer frame. Entities moving > 3 m per tick are treated
  as teleported (not interpolated). If no newer frame exists we **extrapolate** along velocity
  for at most `max_extrapolation_ms` (100 ms), then freeze.
- **View time:** every input carries `view_tick + view_frac/256 = render_tick`, i.e. exactly
  the server tick the player was looking at. The server rewinds hitboxes to this time.

## Lag compensation (`server::lagcomp`)

`LagCompHistory` keeps one `HitboxFrame { tick, hitboxes }` per tick for 1 s
(`LAG_COMP_HISTORY_MS`). Hitboxes (`shared::sim::Hitbox`) are capsules/spheres tagged with a
`HitTarget` (`Player(id)` or `Entity(NetId)`) and a `HitPart` (`Body`, `Head`, `Weak(i)`);
players record a body capsule + head sphere. `rewind(view_time_ticks, max_rewind_ms)` clamps
the time to `[newest − lag_comp_max_ms, newest]` (and the recorded range), then interpolates
every target between the two recorded ticks around it (same ticks the client interpolated
between). `Rewound::ray_hit` finds the nearest hit, ignoring the shooter.

Weapons run inside `SimWorld::step`, which receives the history as
`Option<&dyn HitboxHistory>`; a hitscan shot from `input` calls
`history.rewind_hitboxes(input.view_time_ticks(), tuning.net.lag_comp_max_ms)`. Parry timing
should validate against the same rewound time with `parry_tolerance_ms` extra.

## Tuning

The server loads `assets/tuning.ron` (`Tuning::load_or_default`), watches it with
`engine::hotreload::FileWatcher` and on change reloads it (a file that fails to parse keeps the
previous values) and broadcasts `ServerMessage::Tuning`. Clients always predict with the
server's tuning (`NetClient::tuning()`), and the client's own `NetTuning` (interpolation,
smoothing) comes from the server too. `Server::set_tuning` / `ServerCommand::SetTuning` push
changes programmatically (e.g. a listen-server debug UI).

## Network simulator (`server::netsim`)

`NetSimConfig { latency_ms, jitter_ms, loss_percent, duplicate_percent, seed }`, CLI form
`--netsim 100,20,2[,dup]`. Applied **per direction**: `latency_ms` is one-way, so
`--netsim 100,20,2` gives ≈ 200 ms ping. Each packet is delayed `latency ± uniform(jitter)`
(packets can be reordered), dropped with `loss_percent`, duplicated with `duplicate_percent`
(netcode's replay protection rejects the copy and logs it).

Design: a `NetSim` is a delay queue of raw datagrams. `UdpLink` (used by both the server and the
client transport) owns one for outgoing and one for incoming packets, **between the UDP socket
and netcode**, so everything (handshake, keep-alives, renet acks, both channels) is affected, in
both directions, on the client (`ClientConfig::netsim`, `--netsim`), the server
(`ServerConfig::netsim`, `velocide-server --netsim`) or both. It runs on the transport's own
clock (advanced by `update(dt)`), so tests on a virtual clock are deterministic with a fixed
seed. When packets are in flight the server host loop polls every 1 ms.

## Sessions (`client::net::session`)

| | server | client connects to |
|---|---|---|
| `Session::single_player(level, name)` | `ServerHandle::spawn(ServerConfig::single_player)` on `127.0.0.1:0` | the actual port |
| `Session::host(port, level, max_players, name)` | listen server on `0.0.0.0:port` (thread), lobby, host token | loopback, same port |
| `Session::join(addr, name)` | remote | `addr` |
| `Session::start(SessionSpec, ClientConfig)` | general form (netsim etc.) | |

**Plug-in seam for Steam / matchmaking:** add a `JoinTarget` variant (e.g. `SteamLobby(u64)`),
implement `client::net::ClientTransport` (and `server::ServerTransport` for hosting; renet's
`renet_steam` has exactly this shape) and build with `NetClient::with_transport` /
`Server::with_transport`. Matchmakers that hand out an address (+ optionally a netcode secure
connect token) resolve to `JoinTarget::Address`.

## Using it from the game

```rust
let mut session = Session::single_player("proving_ground", "Shrike")?; // or host / join
let mut timestep = FixedTimestep::new(60.0);
loop {
    let net = session.client_mut();
    net.update(frame_dt);                                  // receive, reconcile, interpolate
    for event in net.drain_events() { /* Spawned{yaw} → camera, Chat, PlayerJoined, Disconnected… */ }
    timestep.set_hz(net.tick_rate() as f64);
    for _ in 0..timestep.advance(frame_dt.as_secs_f64()) {
        net.tick(build_input());                           // predict + send (seq/view time filled in)
    }
    let me = net.local_render_position(timestep.alpha()); // predicted + smoothed
    for e in net.remote_entities() { /* draw at e.position, e.yaw */ }
    let overlay = net.stats(); // rtt, loss, bytes/s, snapshot rate, corrections, interp delay,
                               // server_position (ghost) vs predicted_position
}
```

`velocide` client flags (implemented by the game binary): `--connect <ip:port>`, `--host [port]`,
`--name <name>`, `--netsim <l,j,loss>`; no flags = single-player.
`velocide-server` flags: `--port`, `--bind`, `--level`, `--max-players`, `--mode`,
`--frag-limit`, `--time-limit`, `--friendly-fire`, `--autostart`, `--no-lobby`, `--tick-rate`,
`--snapshot-rate`, `--netsim`, `--tuning`, `--no-watch`, `--no-delta`, `--name`.
`scripts/local_test.sh [--netsim 100,20,2] [--clients N] [--release]` builds and launches a
dedicated server plus clients; Ctrl-C stops everything.

## Validation of client data

Never trusted: inputs are sanitised (above), seq windows enforced, ≤ 16 inputs per packet,
≤ 1 simulated input per tick on average (token bucket), ≤ 64 queued; acks only move forward and
must be ≤ the current tick; chat is size-limited, control characters stripped, rate-limited;
names sanitised (printable, ≤ 24 chars, made unique); per update at most 32 reliable / 128
unreliable messages per client are processed (the rest dropped); undecodable messages are
counted and a client sending 200 is kicked. Clients never send positions or hit claims.

## Measurements (from the tests, virtual clock)

`client/tests/net_loopback.rs` — 2 clients, 100 ms one-way latency (≈ 205 ms RTT), ±20 ms
jitter, 2 % loss both directions, 8 s of scripted running/turning/jumping/strafing:

| | |
|---|---|
| prediction corrections | **0** for both clients; final predicted state == server state (bit-exact) |
| inputs lost to packet loss | **0** (netsim dropped ~1.5 % of each client's packets; every input arrived ~6×) |
| B's interpolated view of A vs A's true server position at the displayed tick | mean **1.0 cm**, p95 **3.6 cm**, max 0.42 m (a landing corner between two snapshots: the placeholder movement drops from ~60 to 16 m/s in one tick) |
| display delay of remote players | ≈ 193 ms behind the server = 100 ms latency + ≈ 104 ms interpolation delay |
| bandwidth per client while playing | down ≈ 4.7 KB/s, up ≈ 11.3 KB/s (60 input packets/s × 6 inputs) |
| snapshot size | 64.6 B average message, 28.2 B entity block (2 players) |

`delta_compression_reduces_bandwidth` — 2 moving players + 4 idle bots + 1 door, 30 Hz, no
netsim, same session with and without delta:

| | entity block | snapshot message | server→client on the wire |
|---|---|---|---|
| delta | 28.2 B | 63.4 B | 4225 B/s per client |
| full | 101.5 B | 134.9 B | 6386 B/s per client |
| saving | **72 %** | 53 % | **34 %** (rest is the unquantised own state, headers, netcode/renet overhead, acks) |

`server_impulse_is_corrected_smoothly` — a server-side (4, 9, 0) m/s impulse at 100 ms ping:
1 correction, 0.16 m error at the acknowledged input, predicted position shifted 1.15 m, no
snap, largest visible change of the drawn position caused by the correction 2.7 cm per 2 ms
frame (the offset decaying), final state identical to the server.
`inputs_survive_heavy_loss` — 10 % loss: 0 inputs skipped, 0 corrections.

Run them with `cargo test -p client --test net_loopback -- --nocapture`.

## Adding gameplay entities to snapshots

1. Store the entity in `SimWorld` with a `NetId` from `SimWorld::alloc_net_id()`.
2. Add a `kind` constant in `shared::protocol::snapshot::kind` (stable; protocol!) and
   document what `owner`, `flags`, `state`, `health`, `aux` mean for it.
3. In `SimWorld::replicate`, `frame.insert(&EntityState { id, kind, position, velocity, yaw,
   pitch, flags, state, health, aux, .. })`. Removal is automatic (absent ⇒ removed in the
   delta). Fields that rarely change cost nothing in deltas.
4. If it can be shot, add its hitboxes in `SimWorld::hitboxes` with `HitTarget::Entity(id)`;
   lag compensation then covers it.
5. Owner-only data (health, ammo, style meter) goes in `OwnState`, not in entities.
6. Clients get it interpolated in `NetClient::remote_entities()`; fast projectiles that should
   be drawn at present time can instead be extrapolated from `position + velocity · Δt` using
   `NetClient::server_tick_estimate()`.
7. Bump `PROTOCOL_VERSION` when the meaning of existing fields changes.

## Known limitations / future work

- No interest management: every client gets every entity (fine for ≤ 8 players and arena-sized
  levels; per-client frames are already supported by the per-client baseline ring).
- Inputs are sent unquantised (yaw/pitch f32) with 6× redundancy: ≈ 11 KB/s upstream. Quantising
  angles in `sanitize_input` and delta-packing the redundant inputs would roughly halve it.
- Door open/close reaches clients with the snapshot, slightly after the server toggled it, so a
  player touching a door exactly as it changes may get a small correction.
- Remote players are shown ≈ latency + 100 ms in the past (inherent to interpolation); lag
  compensation makes hits fair, but projectiles/parries need the tolerance windows in tuning.
- Clock offset uses a symmetric EMA; a sudden latency increase is followed within ~1 s.
- `Unsecure` netcode: no encryption keys / auth; switch to `Secure` connect tokens from a
  matchmaker for public servers.
