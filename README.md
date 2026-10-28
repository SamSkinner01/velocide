# VELOCIDE

A fast, style-driven retro multiplayer FPS on a small custom Rust engine. The Foundry never
stopped pouring: every scrap of heat left in the world has been cast into its wax-and-slag
*Moulds*. You play a *Shrike*, a masked hunter whose life is a flame burning in a lantern in its
chest, fast enough to tear that heat back out of them up close.

You can play single-player or online co-op (2–4 players) through the campaign levels **Crucible
of Veins**, **Spire of Wicks** and **The Drowned Chandlery**, survive endless waves together in
**Grind**, or fight each other (up to 8 players) in **Deathmatch** and **Gun Game** on the arenas
**Cistern Nine**, **Knell Yard**, **Ingot Run**, **Tallow Works**, **Wickmarket** and **Furnace
Ring**. Pick your own loadout of guns, melee weapons, abilities and a knack, and your Shrike's look. Everything is
original and procedural: the art, models, sound effects and music are all generated in code.

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

**Hosting from the game:** go to *Host Game* in the main menu, pick the mode (Campaign co-op,
Grind, Deathmatch or Gun Game), the level and, for co-op modes, the difficulty, then start. You
land in the **warm-up**: the Vestry, a hall with a movement course, a firing range of training
Effigies and a console that summons practice packs of Moulds. Run around while everyone joins.
Tab opens the lobby panel (players, chat, match settings, Customise, Loadout); it shows the
address friends should use, such as `192.168.1.20:47420`. F1 readies you up. When everyone is
ready, the host presses *Start*; after the match everyone comes back to the warm-up.

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
cargo run -p server --release --bin velocide-server -- --mode grind --level furnace_ring --difficulty hard
cargo run -p server --release --bin velocide-server -- --mode gungame --level wickmarket
```

Without the source code, download the server from a release instead
(`velocide-server-linux-x86_64` or `velocide-server-windows-x86_64.exe`, see below) and give it
the same flags.

Modes: `campaign`, `deathmatch`, `gungame`, `grind`. Difficulty (co-op modes): `easy`, `normal`,
`hard`, `brutal`, `inferno`. `--no-warmup` keeps players in a menu-style lobby instead of the
Vestry.

On a dedicated server, the first player to join becomes the lobby host. With `--autostart`, the
match starts as soon as everyone is ready. Dev chat commands (`/god`, `/spawn`, `/tp`...) are
off on dedicated servers; `--dev-commands` turns them on for the lobby host only. In
single-player they are always available, and on a listen server only the host can use them.
`--help` lists every flag.

## Sharing the game with friends

Friends don't need the source code or any tools. Send them the link for their system:

- Windows: <https://github.com/SamSkinner01/velocide/releases/latest/download/velocide-windows-x86_64.exe>
- Linux: <https://github.com/SamSkinner01/velocide/releases/latest/download/velocide-linux-x86_64>

These always fetch the newest release from the
[Releases page](https://github.com/SamSkinner01/velocide/releases). The game is that one file,
with everything it needs built in: nothing to unpack, no folder to keep next to it. On Windows,
the first launch shows a SmartScreen warning because the exe isn't signed ("More info" → "Run
anyway"); on Linux, make the file executable first (`chmod +x velocide-linux-x86_64`). It runs
on any Linux distro with glibc 2.31 or newer, so about 2020 or newer. Each release also has a
`PLAY.txt` with the controls and how to host and join. There is no macOS build; it would have
to be built on a Mac.

Anyone who played an older version from a zip: put the new file in a folder of its own, or
delete the old `assets` folder. An `assets` folder next to the game still overrides the files
built into it (that is how development works), so an old one would bring back old levels and
models.

Everyone in a game, the server included, must run the same version (the release's name, such
as `build-42`). The game checks for a newer release when it starts and offers to update itself
from the main menu (`docs/distribution.md`), so friends download it by hand only once.

The releases are made by CI: every push to `main` becomes a release `build-<n>`, and every `v*`
tag a release under its name, once fmt, clippy, the tests and both builds pass
(`.github/workflows/release.yml`, `docs/hosting.md` "Releases from CI"). Pull requests get the
same checks and builds, without a release.

To build the same files yourself:

```sh
scripts/package.sh            # or: scripts/package.sh linux / scripts/package.sh windows
```

The builds run in containers via podman or docker, so no extra toolchains are needed. The
output lands in `dist/`: `velocide-windows-x86_64.exe`, `velocide-linux-x86_64`, the dedicated
servers `velocide-server-windows-x86_64.exe` and `velocide-server-linux-x86_64`, and `PLAY.txt`.
Their version is the git commit (`-dirty` with uncommitted changes).

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
| Knack (your Q ability: Thorn, Kindle Flask, Snare, Soot Pot, Tallow Skin, Surge) | Q | Y / North |
| Use / revive a teammate | E | X / West |
| Weapon slots (your loadout) | 1–4 (press again to switch between the build's two abilities) | D-pad |
| Next / previous weapon | mouse wheel | D-pad right / left |
| Cycle fire mode | middle mouse | D-pad up |
| Scoreboard (warm-up: lobby panel) | Tab | Select |
| Ready up (warm-up) | F1 | D-pad down |
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
- **Parry.** Punch an incoming projectile just before it hits you (parryable ones wear a ring
  that tightens and strobes once they're in reach). It flies back at double damage and refills
  your wick. A Trimmer slash deflects them too. Punch a **Gaoler** while its shield rim strobes white-hot to
  stun it, or swat a diving **Harrier** out of the air.
- **Knacks** (Q). Pick one in the loadout:
  - *Thorn* (solo and co-op only; in PvP you carry Surge instead): a relay spike that plants
    itself in a wall or floor. Shoot it with the Quill and the shot jumps to the nearest enemy
    (branded ones first); chaining thorns multiplies the damage.
  - *Kindle Flask*: a healing pool for you and your teammates.
  - *Snare*: a wax trap that pins and hurts the first enemy to step on it.
  - *Soot Pot*: a cloud Moulds can't see through.
  - *Tallow Skin*: take much less damage and no knockback for a few seconds.
  - *Surge*: a burst of speed along your look.
- **Style.** Mix weapons, kill in the air, parry and ricochet to climb the ranks from DULL up to
  VELOCIDE. Repeating the same weapon goes *stale* and earns less.

### Loadouts and builds

Choose your weapons in **Loadout** (main menu, lobby, pause menu, warm-up): up to four on the
slot keys 1–4, plus your **knack** on Q. Each weapon's base stats and primary fire are fixed;
you pick its **two abilities** (the alt fire) from the pool of its class. The screen shows the
selected weapon turning in 3D with its PvE and PvP numbers, and both abilities in boxes that
explain what they do and how to use them. Presets save whole loadouts. In a match a change
applies at your next respawn (at once in the warm-up). Gun game hands out its own weapons and
keeps your knack.

| class | weapons | ability pool |
|---|---|---|
| Precision | **Quill** (spike-caster, shots ricochet off Thorns), **Lancet** (rapid needle pistol) | Brand, Splinter, Skewer, Flurry |
| Scatter | **Bellows** (twin-barrel blast cannon), **Thresher** (drum-fed auto-scattergun) | Gust, Both Barrels, Slug, Flak |
| Rapid | **Arclash** (chaining arc caster), **Riveter** (hot-rivet driver), **Kiln** (ember sprayer, sets targets alight) | Wisp, Discharge, Volley, Flare |
| Explosive | **Tumbler** (bouncing bombs), **Cinderbox** (three-rocket salvo) | Breach, Gravewell, Limpet, Mortar |
| Draw | **Longwick** (recurve bow: hold to draw, release to loose) | Brand, Triple Nock, Pinning Arrow, Ember Arrow |
| Melee | **Trimmer** (hooked wick-shears: fast slashes that also deflect projectiles), **Tamper** (foundry ramming hammer: slow, heavy, wide) | Lunge, Whirl, Hurl, Upswing |

The abilities: *Brand* (the target takes extra damage from everyone), *Splinter* (shrapnel that
reflects off walls), *Skewer* (hold to charge a piercing shot), *Flurry* (three-shot burst),
*Gust* (air blast that turns projectiles around), *Both Barrels* (double blast, blast-jump),
*Slug* (one heavy accurate shot), *Flak* (airbursts into a ring of pellets), *Wisp* (ball
lightning), *Discharge* (charged shock nova), *Volley* (ten shots at once), *Flare* (burning patch
on the ground), *Breach* (point-blank blast-jump), *Gravewell* (pulls enemies in, implodes),
*Limpet* (sticky charges: tap to throw, hold to detonate), *Mortar* (heavy lobbed shell), *Triple
Nock* (three arrows), *Pinning Arrow* (pins enemies, slows players), *Ember Arrow* (bursts into
flame), *Lunge* (a low leap that strikes the first thing you reach), *Whirl* (a spin that hits
everything around you), *Hurl* (throws a spinning blade or hammer head), *Upswing* (launches
what it hits into the air). **Burn** stacks damage over time; **Pinned** roots a Mould for a
moment.

PvP and PvE are balanced separately for every weapon and ability (`docs/arsenal.md`,
`docs/balance.md`): in PvP a skilled player kills in about 0.8–1.8 s, and only headshots with a
fully drawn bow, a Slug, a fully charged Skewer or a point-blank Bellows blast kill in one hit.
`cargo run -p shared --release --example balance_report` prints every weapon's damage, DPS and
time to kill against each Mould and a PvP player, measured in the real simulation.

### The Moulds

| Enemy | What it does | How to beat it |
|---|---|---|
| **Gnawer** | knuckle-walking pack hunter; bites, pounces at runners | head is big and low |
| **Censer** | brazier-headed lobber; 3 arcing ember bombs, swings at rushers | punch the embers back; shoot the brazier |
| **Harrier** | wax-winged flyer that dives at you | dodge sideways, or punch it mid-dive |
| **Gaoler** | 3.7 m jailer with a cell-door shield | parry the bash, jump the chain sweep, hit the lantern on its back |
| **Heron** | stilt-legged sniper; red aim laser locks white-hot just before it fires | dash out of the laser after it locks; its lens is high above cover |
| **Bellwether** | hangs back and rings its bell: nearby Moulds are frenzied (faster, harder-hitting) | hunt it down; shoot the bell during the wind-up to stagger it |
| **Snuffer** | leaps onto you and smothers your wick until you get it off | punch it off (or dash, or pin it) |
| **Smelter** | slow heavy that pours molten slag lines along the floor | step off the glowing line before it ignites; hit the crucible hatch |

Moulds are drawn big and their hit shapes reach past the model, so your shots land when they
look like they should. Any Mould can come as an **elite** (more health, faster attacks, a searing glow). The Vestry's
**Effigies** are training dummies: they show your hits and never die.

Moulds come after you: they leap gaps, ledges and deep water (a crouch, then the leap), ride
jump pads, pace the edge when they can't reach you, and one stranded for a few seconds melts
into a wax puddle and rises out of a new one near you. Water and lava still kill them: knock
them in.

### Modes

- **Campaign** (1–4 players): Crucible of Veins, Spire of Wicks, The Drowned Chandlery.
- **Grind** (1–4 players, any arena): endless waves that grow and bring tougher Moulds; every
  5th wave is heavy, every 10th has a boss elite. Fallen players return at the next wave; the
  run ends when everyone is down at once. Your best wave per arena and difficulty is saved.
- **Spectating** (co-op): while you're down or out, watch a teammate through their eyes (Jump:
  chase camera; Fire / Alt fire: next / previous teammate).
- **Difficulty** (campaign and grind): Easy, Normal, Hard, Brutal, Inferno. On top of it the
  Moulds scale with the number of players (more of them, tougher and more aggressive).
- **Deathmatch** (2–8 players): frags with a frag and time limit.
- **Gun Game** (2–8 players): every kill with your current weapon moves you up a ladder of all
  ten guns; the final tier is the **Trimmer**, and a kill with it wins. Getting punched knocks
  you down a tier.
- **Play of the game** (PvP modes): when the match ends, everyone watches the best highlight
  replayed from that player's eyes before the results. The kill feed shows each kill's distance.

## Settings

Settings are saved to `~/.config/velocide/settings.ron`. They include every retro effect
(internal resolution, vertex wobble, affine textures, dithering, colour depth, fog, vertex
lighting), screen shake, flashes, hitstop, FOV, sensitivity, volumes and key bindings.

**Customise** (main menu or lobby) sets your Shrike's mask, crest, cloak, emblem, colours and
wick flame, with a turning preview; everyone sees it. **Loadout** is described above.

**Accessibility** (Settings → Accessibility): a colour-vision mode for protanopia,
deuteranopia or tritanopia with a strength slider. It corrects the 3D view, switches the HUD,
team colours, enemy wind-ups, the parry window, the Heron's laser and enemy projectiles to a
palette that stays distinguishable, and adds shape cues (player marker shapes, a "!" diamond
over parryable enemies, +/− health popups).

## For developers

- **Code and design:** read `DESIGN.md`, plus `docs/*.md` for each subsystem.
- **Tuning while running:** gameplay numbers live in `assets/tuning.ron` (built into release
  binaries; a file on disk overrides the built-in one), which is watched while the game runs. Edit and save it, and the server reloads it and sends it to every client. The
  F3 overlay has an in-game editor for the movement values.
- **F3 overlay:** FPS, ping, packet loss, bandwidth, prediction corrections, and a "ghost"
  outline of where the server thinks you are.
- **Simulated lag:** `--netsim 100,20,2` adds 100 ms one-way latency, ±20 ms jitter and 2 %
  packet loss.
- **Local multiplayer test:** `scripts/local_test.sh --netsim 100,20,2` starts a dedicated
  server plus two clients with one command.
- **Dev chat commands** (in a match; in hosted games only the host can use them):
  `/spawn gnawer 5`, `/spawn gaoler 1 elite`, `/enemies`, `/god`, `/give`, `/killall`, `/melt`,
  `/loadout longwick:pinning,ember kiln trimmer knack:snare`, `/needle` or `/ember` (parry practice),
  `/encounter 1`, `/goto exit`, `/goto checkpoint 3`, `/tp x y z`, `/help`.
- **Tests:** `cargo test --workspace`. Lint: `cargo clippy --workspace --all-targets`, which
  must stay at zero warnings. CI runs both, and `cargo fmt --check`, on every pull request and
  push to `main`.
