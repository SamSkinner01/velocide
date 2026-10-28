# Online play without port forwarding: the rendezvous service

A player can host from their own PC and friends join with a **join code** (or find the game in
the **server browser**) with nobody forwarding a port. One small public service makes that
possible: `velocide-rendezvous`, a single binary on a cheap VPS. It does three things:

1. **Lists games and hands out join codes.** A host registers its game and gets an 8-symbol
   code (`ABCD-EFGH`); public games also appear in the browser.
2. **Introduces joiners to hosts** so both send packets at each other at the same moment and
   their routers let the traffic through (UDP hole punching). Most home connections then talk
   directly: the service carries none of the game's traffic.
3. **Relays** when that doesn't work (strict NATs, carrier-grade NAT, or a player who chose to
   hide their address): it forwards the game's packets between the two, like a minimal TURN
   server.

```
  host's PC (behind NAT)            rendezvous service (VPS)           joiner's PC (behind NAT)
  ──────────────────────            ────────────────────────           ────────────────────────
  Register (from the game socket) ─▶ sees the host's public address
                                    gives it code ABCD-EFGH
                                    ◀───────────────────────────────── Join ABCD-EFGH
  ◀── Introduce: joiner is at X ─── tells each side about the other ──▶ JoinOk: host is at Y
  Punch ─▶ X                                                            Punch ─▶ Y
  ◀════════════════ netcode, directly host ↔ joiner (both NATs now let it through) ══════▶
       if the direct way doesn't connect within 3 s (or either side asked for relay only):
  ◀════════ netcode wrapped in Relay packets ═════▶ service ◀═════ Relay ═════════════════▶
```

The game's own protocol is unchanged: netcode connects end to end (the service never sees
inside netcode's packets), and the **host decides who gets in**: it admits only joiners the
service introduced (the gate, below).

## For players

- **HOST GAME → ONLINE**: *OFF* (LAN, or a forwarded port, as before), *JOIN CODE* (default:
  the lobby shows a code, friends type it in), *PUBLIC* (also in the browser). *HIDE MY IP
  (RELAY)*: joiners always go through the relay and never learn your address.
- **JOIN GAME**: type the code and press *JOIN BY CODE*, or *BROWSE GAMES*. *HIDE MY IP
  (RELAY)* does the same for a joiner. Joining by address still works.
- The lobby shows `JOIN CODE ABCD-EFGH` top right (and the LAN address under it). A kicked
  player can't come back to that game, by code or by address.
- **SETTINGS → GAMEPLAY → ONLINE SERVICE** overrides the service built into the game (empty:
  the built-in one). Builds without one (dev builds, or before the repository variable is set)
  host on the LAN only and say so on the host and join pages.
- Command line: `--host --online code|public|off`, `--join ABCD-EFGH`, `--relay-only`,
  `--rendezvous HOST:PORT`. Dedicated server: `--online`, `--rendezvous HOST:PORT`,
  `--private`, `--relay-only`, `--online-only` (only introduced joiners from the internet).

## How a join works

1. The joiner's game asks the service for a **cookie** (`Hello`), proving it receives packets
   at its address, then sends `Join { code, protocol, three netcode client ids, LAN address,
   relay_only }` from the same UDP socket its netcode will use.
2. The service checks the code (rate-limited per IP), the protocol version and the host's bans,
   opens a relay session, and sends:
   - the host an `Introduce` (MACed with the host's secret, resent every 250 ms until
     acknowledged): the joiner's public address (unless relay only), its LAN address (only if
     both share a public IP), its three client ids, the host's relay credential and a ban tag;
   - the joiner a `JoinOk`: the host's public address (unless relay only), the host's LAN
     address (same public IP only) and the joiner's relay credential.
3. The host punches toward the joiner; the joiner tries, in order, **LAN** (1.5 s), **direct**
   (3 s, punching every 100 ms) and the **relay** (10 s), each with its own netcode client and
   client id. The first that connects wins.
4. The host's gate admits the connection because its client id is one the introduction
   announced, from an address the introduction allows (the joiner's IP, its LAN IP, or the
   relay session). The introduction is used up: the other two ids no longer get in.

Relayed peers appear to netcode under stand-in addresses in `100::/64` (the IPv6 discard
prefix, so no real packet ever comes from one): `rendezvous::relay_addr(session)`. The
transports turn netcode's packets to such an address into `Relay` packets to the service and
back.

## Protocol (version 1)

Every datagram: `FF 56 52 5A` (magic; `0xFF` is never the first byte of a netcode packet, so
one socket carries both), version `01`, a kind byte, then the fields, little-endian. Strings are
a length byte plus printable ASCII; addresses a tag (0 none, 4, 6) plus IP and port. Decoding
is strict (`rendezvous::proto::decode`): wrong lengths, unknown flags or kinds, out-of-range
values and trailing bytes reject the whole datagram. Control messages are at most 1200 bytes;
`Relay` carries up to 1400 bytes of payload (netcode's largest packet).

| kind | message | direction | cookie | notes |
|---|---|---|---|---|
| 01 | `Hello { nonce }` | C→S | – | padded to 64 bytes |
| 02 | `Cookie { nonce, cookie, observed }` | S→C | | 16-byte cookie; `observed`: the sender's address as the service sees it |
| 03 / 04 | `Probe` / `ProbeReply { version }` | C→S / S→C | – | health check (deploys); padded to 64 |
| 05 / 06 | `Error { nonce, code }` / `Ok { nonce }` | S→C | | codes: BadCookie, RateLimited, UnknownHost, UnsupportedVersion, Busy, Refused |
| 10 / 11 | `Register { resume?, lan?, info, bans }` / `Registered { host_id, code, secret }` | C→S / S→C | yes | `resume` = (host_id, code, secret) to get a listing back after a restart |
| 12 / 13 | `Heartbeat { host_id, secret, info }` / `Unregister` | C→S | yes | every 2 s; a listing without one for 10 s is dropped |
| 14 / 15 | `Introduce { …, mac }` / `IntroAck` | S→C / C→S | ack: yes | see above |
| 16 | `Ban { host_id, secret, tag }` | C→S | yes | |
| 20 / 21 | `List { protocol, page }` / `ListPage { page, pages, total, entries }` | C→S / S→C | yes | 14 public games of that protocol per page |
| 22 / 23 / 24 | `Join` / `JoinOk` / `JoinFailed { reason, host_build }` | | yes | reasons: NotFound, VersionMismatch, Banned, Busy, RateLimited |
| 30 | `Relay { session, payload }` | both | – | forwarded only between the session's two ends |
| 31 / 32 / 33 | `RelayBind { session, role, expires, host_id, cred }` / `RelayBound` / `RelayUnknown` | | bind: yes | (re)bind one end of a session |
| 40 | `Punch { join_id }` | C↔C | – | opens NATs; ignored on arrival |

Requests carry a random 64-bit nonce and replies echo it: the game accepts a reply only from
the service's address and with the nonce of a request it has outstanding. Retried requests reuse
their nonce, and the service answers a retried `Register` or `Join` with the same reply (15 s),
so a lost reply never makes a second listing or relay session.

Changing the protocol: bump `proto::VERSION`. A service answers other versions with
`UnsupportedVersion` (the game then says "update the game"), so keep the service able to speak
the version of every release players still run, or accept that older releases lose online play.

## Security

**Threat model.** The service is a public UDP endpoint anyone can send anything to, from any
spoofed address. Hosts and joiners are untrusted to each other and to the service. Like the
game's own transport (netcode in unsecure mode), the protocol is not encrypted: someone *on the
network path* (same Wi-Fi, an ISP) can read codes and credentials and impersonate a player. It
is built so that nobody *off* the path can.

The rules, and where they are enforced:

- **No reflection or amplification** (`Service::reply_unverified`). A sender that hasn't proven
  its address gets at most one reply per datagram, never bigger than the datagram, from a
  global budget (2000/s). `Hello` and `Probe` are padded to 64 bytes so their answers fit. A
  spoofed flood can't make the service send more to a victim than the attacker sent.
- **Address proof before state** (`Keys::cookie`). Everything that creates or changes state
  needs a cookie: an HMAC of the sender's IP and port and the minute, valid for 60–120 s, sent
  only to that address. Per-IP rate limits apply only after this check, so their table can't be
  filled from spoofed addresses (and it is bounded: 65 536 IPs).
- **Ownership** (`Keys::host_secret`, `Service::owned`). A listing's heartbeats, unregister,
  bans and acknowledgements need its 32-byte secret, compared in constant time. Introductions
  to a host carry an HMAC under that secret; the host ignores any without a valid one, any not
  from the service's address, and replies without its own outstanding nonce.
- **Join codes are keys** (`code.rs`, `IpRates`). 40 random bits from the OS random source;
  joins are limited to a burst of 5 then one every 2 s per IP. A private game isn't listed
  anywhere, and the service never logs codes.
- **The gate** (`HostAgent::on_receive`, `HostAgent::admit`; `OnlineConfig::gate`, on for
  online listen servers). From the internet, only packets from introduced joiners reach netcode
  at all, and only a connection with an announced client id is admitted, once. Knowing the
  host's address isn't enough to get in, so a code is a real access key, and kicks hold: a kick
  bans the joiner's IP at the gate and, through a ban tag (an HMAC of host and IP that tells the
  host nothing), at the service. Loopback (the host's own client) and the LAN (private IPv4)
  skip the gate. A dedicated server's gate is off by default (it is joinable by address anyway),
  `--online-only` turns it on.
- **Relays can't be hijacked or turned into proxies** (`Service::relay`, `relay_bind`). A
  session is created only by a join, between that host and that joiner. Packets are forwarded
  only between its two bound addresses. Binding an end (after a restart, or when a NAT changes
  ports) needs that end's credential: an HMAC of session, role, expiry and host, valid 24 h.
- **Bounded resources and bills** (`ServiceConfig`). 1024 listings (4 per IP), 512 relay
  sessions (16 per host, 8 per joiner IP), 16 pending introductions per listing, 64 KiB/s per
  relayed direction (512 KiB burst for level loads), 2 MiB/s and 20 GB per day relayed in all,
  so a heavy user can't run up the VPS's traffic bill. Idle relays close after 30 s.
- **Privacy.** The browser shows only the code, name, mode, level and player count; addresses
  are only given to the two sides of a join, and not at all with *HIDE MY IP*, in either
  direction. A host's LAN address only goes to joiners behind the same public IP, and only if
  it looks like a LAN address (private IPv4, port ≥ 1024): a host can't point joiners at
  arbitrary services.
- **Parsing.** The crate forbids `unsafe` and denies `unwrap`, `expect`, `panic!` and slice
  indexing outside tests; every read is bounds-checked; nothing allocates more than the datagram
  holds. A fuzz test throws 200 000 random and corrupted datagrams at the decoder.
- **What a malicious host can do to joiners** is what any server could already do (the game
  validates and size-limits everything a server sends, docs/network.md); the public browser
  makes strangers' servers one click away, which is why the client side of that boundary was
  part of the audit.

**Not protected:** an on-path attacker (see above), and denial of service by bandwidth (the
service rate-limits work, not the link). A botnet guessing codes from many IPs is slowed by
the per-IP limit but not stopped; a guessed code lets a stranger into a lobby, where the host
can kick (and so ban) them.

## Restarts

The service keeps no state on disk except its 32-byte **master key** (`--key-file`, made on
first start, 0600). Everything it hands out is an HMAC under keys derived from it, so after a
restart (a deploy) it can check what clients present without having stored it:

- hosts get `UnknownHost` on their next heartbeat and register again with `resume`: same
  listing id, same **code**; they also rebind their relay sessions at once;
- relayed players' next packet gets `RelayUnknown`; both ends rebind with their credentials
  and the game carries on (a hitch of about a second).

With a new key (the file was lost) hosts get new codes and relayed games end.

## Running it

`velocide-rendezvous --help` lists the flags (limits above, `--bind`, `--key-file`,
`--exit-when-replaced`, `--probe HOST:PORT [--expect VERSION]`). It needs nothing but UDP
port 47400; the release asset `velocide-rendezvous-linux-x86_64` is a standalone binary.

The game finds the service through `VELOCIDE_RENDEZVOUS` (`host:port`) compiled into the game
and the dedicated server (`rendezvous::BUILT_IN_SERVICE`), the repository variable of the same
name in CI; the player's setting or `--rendezvous` overrides it.

## Deploying

Like the chess project, CI updates the VPS on every release. Unlike it, the key CI holds can do
nothing but ask the VPS to install a release from GitHub:

```
release job publishes build-<n>  ─▶  deploy job: ssh velocide-deploy@VPS "deploy build-<n>"
                                      authorized_keys: restrict,command="/usr/local/bin/velocide-deploy"
                                      └─ checks the request is "deploy <tag>" (never runs it)
                                      └─ downloads that release's assets from github.com over
                                         HTTPS, checks GitHub's SHA-256 digests
                                      └─ /opt/velocide/deploy/releases/<tag>/, flips `current`
                                      └─ waits for the service to answer --probe as <tag>,
                                         else flips back
services: --exit-when-replaced notices `current` moved, exits, systemd starts the new binary
          (the online service at once; the dedicated server once nobody is connected)
```

A leaked deploy key can at most make the VPS reinstall an official release. The deploy user
owns only `/opt/velocide/deploy`, has no sudo, and can't change its own `authorized_keys`
(root owns them). The services run as throwaway `DynamicUser`s with a tight systemd sandbox
(`scripts/deploy/*.service`): no capabilities, read-only system, only IP sockets.

### One-time setup

1. **A VPS** (the smallest is plenty) with Ubuntu or Debian. The droplet that runs the chess
   project works: the service uses UDP 47400 (and the dedicated server UDP 47420), nothing on
   TCP.
2. **A deploy key** on your machine: `ssh-keygen -t ed25519 -N "" -C velocide-deploy -f ~/.ssh/velocide_deploy`
3. **The VPS**, as root:
   ```sh
   scp -r scripts/deploy root@VPS:/root/velocide-deploy
   ssh root@VPS 'bash /root/velocide-deploy/setup-vps.sh --deploy-key "$(cat)"' < ~/.ssh/velocide_deploy.pub
   # add --with-server to also run the dedicated server (docs/hosting.md)
   ```
   It installs the newest release, starts the service and opens the ports in `ufw` if it is on.
   A provider firewall (DigitalOcean: Networking → Firewalls) needs **UDP** 47400 (and 47420)
   inbound too.
4. **DNS**: an `A` record such as `play.example.com` → the VPS, **DNS only** (Cloudflare's
   orange-cloud proxy carries HTTP, not UDP).
5. **GitHub** (repository Settings → Secrets and variables → Actions):

   | kind | name | value |
   |---|---|---|
   | variable | `VELOCIDE_RENDEZVOUS` | `play.example.com:47400` (built into the game) |
   | variable | `DEPLOY_HOST` | `play.example.com` (or the VPS's IP); setting it turns deploys on |
   | variable | `DEPLOY_USER` | optional, default `velocide-deploy` |
   | secret (environment `production`) | `DEPLOY_SSH_KEY` | contents of `~/.ssh/velocide_deploy` |
   | secret (environment `production`) | `DEPLOY_KNOWN_HOSTS` | output of `ssh-keyscan -t ed25519 play.example.com` |

   Under Settings → Environments → `production`, limit deployments to `main` and `v*` tags.
6. Push to `main`. The next release's game has the service built in, and the deploy job puts
   that release on the VPS.

Until `DEPLOY_HOST` is set the deploy job is skipped and nothing else waits for it. Logs:
`journalctl -u velocide-rendezvous -f` (counters every 5 minutes: listings, relays, packets,
malformed, bad cookies, rate limited, refused, joins, relayed MB). Health:
`velocide-rendezvous --probe play.example.com`.

## Code

| where | what |
|---|---|
| `rendezvous/src/proto.rs`, `wire.rs`, `code.rs` | the protocol, the bounded reader/writer, join codes |
| `rendezvous/src/crypto.rs` | key derivation, cookies, secrets, credentials, ban tags (HMAC-SHA256) |
| `rendezvous/src/service.rs`, `limit.rs` | the service without IO; rate limits |
| `rendezvous/src/host.rs`, `join.rs` | the game's side without IO: `HostAgent`, `JoinAgent` |
| `rendezvous/src/browse.rs` | the server browser (own socket) |
| `rendezvous/src/bin/velocide-rendezvous.rs` | the UDP loop, key file, `--probe` |
| `server/src/transport.rs` | `OnlineConfig`; `NetcodeServerTransport::with_online`: demux, gate, relay wrapping, bans |
| `client/src/net/online.rs` | `OnlineClientTransport`: join by code, one netcode client per attempt |
| `scripts/deploy/` | `setup-vps.sh`, the forced command `velocide-deploy`, the systemd units |

Tests: `rendezvous/tests/nat.rs` runs the service, hosts and joiners on a simulated network
with full-cone, port-restricted and symmetric NATs (direct, relay, LAN, relay only, the gate,
bans, restarts with the same and a new key, 20 % loss); `rendezvous/tests/attacks.rs` covers
amplification, cookies, secrets, relay hijacking, budgets, limits and forged introductions;
`client/tests/online.rs` runs it all over real sockets (browser, a direct and a relayed joiner
reaching the lobby, a stranger kept out, a kick that holds).
