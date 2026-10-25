# VELOCIDE

A fast, style-driven retro multiplayer FPS on a small custom Rust engine. You play a *Shrike*,
a blood-fuelled chrome automaton that heals by fighting up close. You can play single-player,
online co-op (2–4 players) through the campaign level **Crucible of Veins**, or deathmatch (up to
8 players) in the arena **Cistern Nine**. Everything is original and procedural: the art,
models, sound effects and music are all generated in code.

## Build and run

You need Rust 1.95 or newer (`rustup update`), a Vulkan, Metal or DX12 capable GPU, and on Linux
the ALSA and udev development packages (Fedora: `sudo dnf install alsa-lib-devel systemd-devel`;
Debian/Ubuntu: `sudo apt install libasound2-dev libudev-dev`).

```sh
cargo run -p client --release                          # main menu
cargo run -p client --release -- --sp crucible_of_veins   # straight into single-player
```

Use `--release`; debug builds work but run slower. The binary is `target/release/velocide`.

## Playing with friends

**Hosting from the game:** go to *Host Game* in the main menu, pick the mode (Campaign co-op or
Deathmatch) and the level, then start. You land in the lobby. It shows the address friends
should use, such as `192.168.1.20:47420`. When everyone has pressed Ready, the host presses
*Start*.

**Joining:** go to *Join Game*, type the host's `ip:port`, and you'll arrive in their lobby. You
can also join a match that is already running.

From the command line:

```sh
velocide --host 47420 --name Sam                 # host (listen server)
velocide --connect 203.0.113.7:47420 --name Alex # join
```

**Over the internet**, the host must make the UDP port reachable:

- Forward **UDP 47420** (or your chosen port) on the router to the host's PC.
- Open it in the host's firewall. On Fedora: `sudo firewall-cmd --add-port=47420/udp`.
  Add `--permanent` to keep the rule after a reboot.
- Alternatively, everyone can join a virtual LAN tool (any mesh VPN). Then use the host's VPN IP
  and no port forwarding is needed.

**Dedicated server** (headless, no player needed on the host machine):

```sh
cargo run -p server --release --bin velocide-server -- --port 47420 --mode deathmatch --level cistern_nine
cargo run -p server --release --bin velocide-server -- --mode campaign --level crucible_of_veins --autostart
```

On a dedicated server, the first player to join becomes the lobby host. With `--autostart`, the
match starts as soon as everyone is ready. Dev chat commands (`/god`, `/spawn`, `/tp`...) are
off on dedicated servers; `--dev-commands` turns them on for the lobby host only. In
single-player they are always available, and on a listen server only the host can use them.
`--help` lists every flag.

## Controls (rebindable in Settings → Controls)

| Action | Keyboard / mouse | Gamepad |
|---|---|---|
| Move | WASD | left stick |
| Look | mouse | right stick |
| Jump (wall jump in the air) | Space | A / South |
| Slide (ground) / Slam (air) | Left Ctrl or C | B / East |
| Dash (3 charges) | Left Shift | LB |
| Fire / Alt fire | LMB / RMB | RT / LT |
| Punch / **Parry** | F | RB |
| Throw Prism (ricochet target) | Q | Y / North |
| Use / revive a teammate | E | X / West |
| Weapons | 1 Lancet, 2 Thresher, 3 Riveter, 4 Cinderbox (press again to cycle alt mode) | D-pad |
| Next / previous weapon | mouse wheel | D-pad right / left |
| Cycle fire mode | middle mouse | D-pad up |
| Scoreboard | Tab | Select |
| Chat | T or Enter | |
| Pause | Esc | Start |
| Restart (solo) | R | |
| Debug overlay | F3 | |
| Fullscreen | F11 | |

### How to play

- **Never stop moving.**
  - Sliding keeps your speed indefinitely on flat ground.
  - Slide-jumps and dash-jumps carry momentum.
  - Slam (crouch in the air) and jump right after landing to bounce much higher.
  - You get 3 wall jumps before you have to touch the ground again.
- **Heal by fighting up close.** Every hit you land sprays blood, and standing in it heals you.
  Part of each hit you take is *hard damage*, which temporarily caps your max health.
- **Parry.** Punch an incoming projectile just before it hits you. It flies back at double
  damage and refills your health. Punching a glowing yellow **Bastion** mid-slam stuns it.
- **Ricochet.** Throw a Prism and shoot it with a hitscan weapon. The shot bounces to the
  nearest enemy, and chaining prisms multiplies the damage.
- **Style.** Mix weapons, kill in the air, parry and ricochet to climb the ranks from DULL up to
  VELOCIDE. Repeating the same weapon goes *stale* and earns less.

## Settings

Settings are saved to `~/.config/velocide/settings.ron`. They include every retro effect
(internal resolution, vertex wobble, affine textures, dithering, colour depth, fog, vertex
lighting), screen shake, flashes, hitstop, FOV, sensitivity, volumes and key bindings.

## For developers

- **Code and design:** read `DESIGN.md`, plus `docs/*.md` for each subsystem.
- **Tuning while running:** gameplay numbers live in `assets/tuning.ron`, which is watched while
  the game runs. Edit and save it, and the server reloads it and sends it to every client. The
  F3 overlay has an in-game editor for the movement values.
- **F3 overlay:** FPS, ping, packet loss, bandwidth, prediction corrections, and a "ghost"
  outline of where the server thinks you are.
- **Simulated lag:** `--netsim 100,20,2` adds 100 ms one-way latency, ±20 ms jitter and 2 %
  packet loss.
- **Local multiplayer test:** `scripts/local_test.sh --netsim 100,20,2` starts a dedicated
  server plus two clients with one command.
- **Dev chat commands** (in a match; in hosted games only the host can use them):
  `/spawn husk 5`, `/spawn bastion`, `/god`, `/give`, `/killall`, `/orb` (parry practice),
  `/encounter 1`, `/goto exit`, `/goto checkpoint 3`, `/tp x y z`, `/help`.
- **Tests:** `cargo test --workspace`. Lint: `cargo clippy --workspace --all-targets`, which
  must stay at zero warnings.
