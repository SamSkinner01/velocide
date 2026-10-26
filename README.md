# VELOCIDE

A fast, style-driven retro multiplayer FPS on a small custom Rust engine. The Foundry never
stopped pouring: every scrap of heat left in the world has been cast into its wax-and-slag
*Moulds*. You play a *Shrike*, a masked hunter whose life is a flame burning in a lantern in its
chest, fast enough to tear that heat back out of them up close.

You can play single-player, online co-op (2–4 players) through the campaign levels **Crucible
of Veins** and **Spire of Wicks**, or deathmatch (up to 8 players) in the arenas **Cistern
Nine**, **Knell Yard** and **Ingot Run**. Everything is original and procedural: the art,
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

**Dedicated server** (step-by-step for a VPS such as a DigitalOcean droplet: `docs/hosting.md`) (headless, no player needed on the host machine):

```sh
cargo run -p server --release --bin velocide-server -- --port 47420 --mode deathmatch --level cistern_nine
cargo run -p server --release --bin velocide-server -- --mode campaign --level crucible_of_veins --autostart
```

On a dedicated server, the first player to join becomes the lobby host. With `--autostart`, the
match starts as soon as everyone is ready. Dev chat commands (`/god`, `/spawn`, `/tp`...) are
off on dedicated servers; `--dev-commands` turns them on for the lobby host only. In
single-player they are always available, and on a listen server only the host can use them.
`--help` lists every flag.

## Sharing the game with friends

You don't need to send friends the source code. Build ready-to-run packages:

```sh
scripts/package.sh            # or: scripts/package.sh linux / scripts/package.sh windows
```

The builds run in containers via podman or docker, so no extra toolchains are needed. The
output lands in `dist/`:

- `velocide-<version>-windows-x86_64.zip`: `velocide.exe`, `velocide-server.exe`, `assets\`,
  `PLAY.txt`.
- `velocide-<version>-linux-x86_64.tar.gz`: the same for Linux. It runs on any distro with
  glibc ≥ 2.30, so about 2020 or newer.

Friends unzip the package and run `velocide` (or `velocide.exe`). `PLAY.txt` inside explains
the controls and how to join. `<version>` is the git commit, and everyone, including the
server, must run the same one. On Windows, the first launch shows a SmartScreen warning
because the exe isn't signed ("More info" → "Run anyway"). There is no macOS build; it would
have to be built on a Mac.

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
| Throw Thorn (ricochet relay) | Q | Y / North |
| Use / revive a teammate | E | X / West |
| Weapons | 1 Quill, 2 Bellows, 3 Arclash, 4 Tumbler (press again to cycle alt mode) | D-pad |
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
- **Heal by fighting up close.** Every hit you land knocks embers out of a Mould; the closest
  Shrike draws them into its wick (*kindling*). Part of each hit you take *smothers* your wick,
  temporarily capping your max health.
- **Parry.** Punch an incoming projectile just before it hits you. It flies back at double
  damage and refills your wick. Punch a **Gaoler** while its shield rim strobes white-hot to
  stun it, or swat a diving **Harrier** out of the air.
- **Ricochet.** Throw a Thorn: it plants itself in a wall or floor. Shoot it with the Quill and
  the shot jumps to the nearest enemy (branded ones first); chaining thorns multiplies the
  damage.
- **Style.** Mix weapons, kill in the air, parry and ricochet to climb the ranks from DULL up to
  VELOCIDE. Repeating the same weapon goes *stale* and earns less.

### The arsenal

| Slot | Weapon | Primary | Alt A | Alt B |
|---|---|---|---|---|
| 1 | **Quill** (spike-caster) | precise hitscan quill | **Brand**: the target takes +35 % damage from everyone for 5 s | **Splinter**: bursts into shrapnel, reflects off walls |
| 2 | **Bellows** (twin-barrel blast cannon) | close-range blast | **Gust**: air blast that shoves enemies and turns their projectiles around | **Both Barrels**: double blast, launches you backwards |
| 3 | **Arclash** (arc caster) | beam that chains between enemies | **Wisp**: slow ball lightning that zaps whatever is near | **Discharge**: hold to charge a shock nova around you |
| 4 | **Tumbler** (bouncing-bomb launcher) | bouncing bombs | **Breach**: point-blank blast (blast-jump) | **Gravewell**: pulls enemies together, then implodes |

### The Moulds

| Enemy | What it does | How to beat it |
|---|---|---|
| **Gnawer** | knuckle-walking pack hunter; bites, pounces at runners | head is big and low |
| **Censer** | brazier-headed lobber; 3 arcing ember bombs, swings at rushers | punch the embers back; shoot the brazier |
| **Harrier** | wax-winged flyer that dives at you | dodge sideways, or punch it mid-dive |
| **Gaoler** | 3.7 m jailer with a cell-door shield | parry the bash, jump the chain sweep, hit the lantern on its back |
| **Heron** | stilt-legged sniper; red aim laser locks white-hot just before it fires | dash out of the laser after it locks; its lens is high above cover |

## Settings

Settings are saved to `~/.config/velocide/settings.ron`. They include every retro effect
(internal resolution, vertex wobble, affine textures, dithering, colour depth, fog, vertex
lighting), screen shake, flashes, hitstop, FOV, sensitivity, volumes and key bindings.

**Accessibility** (Settings → Accessibility): a colour-vision mode for protanopia,
deuteranopia or tritanopia with a strength slider. It corrects the 3D view, switches the HUD,
team colours, enemy wind-ups, the parry window, the Heron's laser and enemy projectiles to a
palette that stays distinguishable, and adds shape cues (player marker shapes, a "!" diamond
over parryable enemies, +/− health popups).

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
  `/spawn gnawer 5`, `/spawn gaoler`, `/enemies`, `/god`, `/give`, `/killall`, `/needle` or
  `/ember` (parry practice),
  `/encounter 1`, `/goto exit`, `/goto checkpoint 3`, `/tp x y z`, `/help`.
- **Tests:** `cargo test --workspace`. Lint: `cargo clippy --workspace --all-targets`, which
  must stay at zero warnings.
