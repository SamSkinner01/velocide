# Hosting a dedicated server (e.g. a DigitalOcean droplet)

The dedicated server is one file: the levels, enemy definitions and tuning are built into it.
It needs no GPU, no audio, no `assets/` folder and no Rust toolchain on the machine it runs on.

## 1. Get the server

**Easiest:** download it from the newest release, straight onto the server:

```sh
ssh root@SERVER_IP
mkdir -p /opt/velocide && cd /opt/velocide
curl -L -o velocide-server https://github.com/SamSkinner01/velocide/releases/latest/download/velocide-server-linux-x86_64
chmod +x velocide-server
```

It needs glibc 2.31 or newer (Ubuntu 20.04, Debian 11 and anything later), so it runs on any
current droplet. Skip to step 3. For a particular release instead of the newest, use
`https://github.com/SamSkinner01/velocide/releases/download/build-42/velocide-server-linux-x86_64`
(the tag is the version; the Releases page lists them).

**Or** build it yourself. `scripts/package.sh linux` makes
`dist/velocide-server-linux-x86_64`, the same file a release carries. Or build just the server
as a fully static binary:

```sh
rustup target add x86_64-unknown-linux-musl        # one time
cargo build --release -p server --bin velocide-server --target x86_64-unknown-linux-musl
```

The output is `target/x86_64-unknown-linux-musl/release/velocide-server`. It is a static
binary, so it runs on any x86-64 Linux, including Ubuntu and Debian droplets.

## 2. Copy it to the server (if you built it yourself)

```sh
ssh root@SERVER_IP 'mkdir -p /opt/velocide'
scp target/x86_64-unknown-linux-musl/release/velocide-server root@SERVER_IP:/opt/velocide/
# or the one scripts/package.sh made:
scp dist/velocide-server-linux-x86_64 root@SERVER_IP:/opt/velocide/velocide-server
```

An `assets/` folder next to the binary (or wherever `VELOCIDE_ASSETS` points) still takes
precedence over the built-in files, one file at a time. You only need one to change gameplay
numbers live (see "Updating" below).

## 3. Open the UDP port

The game uses **UDP** port 47420. Opening TCP is not enough.

- **On the droplet:** run `ufw status`. If it says `active`, run `ufw allow 47420/udp`.
- **In the DigitalOcean control panel:** this only applies if a Cloud Firewall is attached to
  the droplet (Networking → Firewalls). Add an inbound rule: Custom, **UDP**, port 47420, all
  sources.

## 4. Try it by hand

```sh
ssh root@SERVER_IP
cd /opt/velocide && ./velocide-server --port 47420
```

You should see `server listening on 0.0.0.0:47420`. In the game, choose **Join Game** and enter
`SERVER_IP:47420`. Press Ctrl-C on the server to stop it.

## 5. Run it as a service

As a service it keeps running after you log out, starts on boot and restarts if it crashes.

```sh
useradd --system --home /opt/velocide --shell /usr/sbin/nologin velocide
chown -R velocide:velocide /opt/velocide
cat > /etc/systemd/system/velocide.service <<'EOF'
[Unit]
Description=VELOCIDE dedicated server
After=network-online.target
Wants=network-online.target

[Service]
User=velocide
WorkingDirectory=/opt/velocide
ExecStart=/opt/velocide/velocide-server --port 47420
Restart=on-failure
RestartSec=3

[Install]
WantedBy=multi-user.target
EOF
systemctl daemon-reload
systemctl enable --now velocide
```

Useful commands:

- `systemctl status velocide`: is it running?
- `journalctl -u velocide -f`: live log (joins, leaves, matches).
- `systemctl restart velocide`
- `systemctl stop velocide`

## Server options

Add these to `ExecStart`. `velocide-server --help` lists them all.

| Flag | Meaning |
|---|---|
| `--mode campaign` / `--mode deathmatch` | default mode in the lobby (the host can change it) |
| `--level crucible_of_veins` / `--level cistern_nine` | default level |
| `--max-players N` | 8 by default (co-op is capped at 4) |
| `--frag-limit N`, `--time-limit SECONDS` | deathmatch limits |
| `--autostart` | start the match as soon as everyone is ready (no host needed) |
| `--name "Friday Night"` | server name |
| `--dev-commands` | enables `/god`, `/spawn` and so on for the lobby host. Leave this off on a public server. |

How a session runs:

1. The server starts in the **lobby**.
2. The first player to join becomes the host and picks the mode and level.
3. Everyone presses Ready, and the host presses Start.
4. After the results screen, everyone returns to the lobby.

## Updating

Everyone (the server and every player) must run the same version of the game. The game offers
players each new release by itself; the server you update by hand. To the newest release:

```sh
ssh root@SERVER_IP
cd /opt/velocide
curl -L -o velocide-server.new https://github.com/SamSkinner01/velocide/releases/latest/download/velocide-server-linux-x86_64
chmod +x velocide-server.new && mv velocide-server.new velocide-server
systemctl restart velocide
```

Download to a new name and `mv` it over the old one: writing straight into the running binary
fails ("Text file busy"), but renaming over it is fine, and the restart starts the new one.

If you build the server yourself, after pulling or changing code:

```sh
cargo build --release -p server --bin velocide-server --target x86_64-unknown-linux-musl
scp target/x86_64-unknown-linux-musl/release/velocide-server root@SERVER_IP:/opt/velocide/velocide-server.new
ssh root@SERVER_IP 'cd /opt/velocide && mv velocide-server.new velocide-server && systemctl restart velocide'
```

Gameplay numbers can be changed live: copy `assets/tuning.ron` from the repository (at the
server's version) to `/opt/velocide/assets/tuning.ron`, (re)start the server, and edit the file
there. It is reloaded on save and pushed to every connected player; everything else still comes
from inside the binary. Delete it when you update the server, or the old numbers keep
overriding the new version's.

## Releases from CI

`.github/workflows/release.yml` (GitHub Actions, on GitHub's own runners) checks every change
and publishes the releases at <https://github.com/SamSkinner01/velocide/releases>, so nobody
has to run `scripts/package.sh` by hand. `docs/distribution.md` is the contract it keeps with
the game's update check.

| trigger | runs | release |
|---|---|---|
| a pull request (and each push to it) | check, linux, windows | none; the files stay on the run's page for 14 days (*Artifacts*) |
| a push to `main` | check, linux, windows, release | `build-<n>` (`<n>`: the run number), titled with the commit; the newest 10 are kept |
| a `v*` tag (`git tag v1.0 && git push origin v1.0`) | the same | under that tag, kept |
| *Run workflow* on the Actions page | the same, for the branch you pick | only for `main` |

Every release carries the same five files: `velocide-windows-x86_64.exe` and
`velocide-linux-x86_64` (the game), `velocide-server-windows-x86_64.exe` and
`velocide-server-linux-x86_64` (the dedicated server), and `PLAY.txt`. Each new release is
marked *latest*, and the names carry no version, so
`https://github.com/SamSkinner01/velocide/releases/latest/download/<name>` always fetches the
newest: those are the links to give friends. The release's tag is the version the game shows
(`build-42`, `v1.0.0`; also in `PLAY.txt`), and the release points at the full commit it was
built from, which the game compares with its own to offer updates.

The jobs:

1. **check** (Ubuntu 24.04): `cargo fmt --check`, `cargo clippy -D warnings` and
   `cargo test --workspace`.
2. **linux** (an Ubuntu 20.04 container, for glibc 2.31) and **windows** (Ubuntu 24.04, MinGW):
   `scripts/ci/setup.sh` and `scripts/build_dist.sh`, with the version and the commit built in.
   They run next to check, not after it, and keep their files as the run's artifacts.
3. **release** (pushes to `main` and `v*` tags, only when the other three passed):
   `scripts/ci/github_release.sh` creates the release with `gh` at the built commit, marks it
   latest, then deletes the `build-*` releases beyond the newest 10, with their tags. `gh`
   uploads the files to a draft and publishes it once all are up, so nobody sees a half-filled
   release. This is the only job that may write to the repository (`contents: write` for the
   run's own token); the others only read.

A newer push to `main` (or to a pull request) cancels the checks and builds of an older run that
is still going, which then releases nothing. A tag's run is never cancelled, and neither is a
release job once it has started. A failed run releases nothing; *Re-run failed jobs* keeps its
version and replaces the release's files if it already got that far.

How it is built:

- The compiling jobs install with `scripts/ci/setup.sh` (packages and rustup; on a runner itself
  through `sudo`) and build with `scripts/build_dist.sh`. `scripts/package.sh` runs the same two
  scripts in local containers, so a local build and a CI one are made the same way.
- The toolchain is pinned by `RUST_VERSION` at the top of the workflow; bump it with the
  machine you develop on.
- Each compiling job keeps its compiled dependencies and crate downloads in GitHub's cache
  (`Swatinem/rust-cache`), saved only by runs on `main`; pull requests and tags start from
  `main`'s. A new cache is saved when `Cargo.lock` or the toolchain changes. GitHub keeps 10 GB
  of caches per repository and drops the least recently used beyond that.
- Disk: a runner has about 14–20 GB free. The check job builds without debug info, which keeps
  its `target/` at about 4 GB (with line tables, as before, it was about 13 GB); its last step
  logs the disk use. A release build's `target/` is about 1.5 GB. The caches stay well under
  the 10 GB: rust-cache keeps the dependencies only, about 3.3 GB of the check job's and 1.2 GB
  of each build's before compression.
- The binaries are always x86_64. On an arm64 machine (a Mac's Docker runs arm64 Linux
  containers) `setup.sh` and `build_dist.sh` cross-compile the Linux binaries
  (`gcc-x86-64-linux-gnu` and the amd64 builds of ALSA, udev and D-Bus from Ubuntu's amd64
  archive); MinGW cross-compiles to Windows from either. CI doesn't need this; `package.sh` on a
  Mac does.

## Notes

- **Size:** the smallest droplet is plenty. A server tick with 8 players costs about 0.1 ms, and
  each player uses about 10–30 KB/s. Pick the region closest to your group, since ping matters
  more than CPU.
- **No password:** anyone who knows the IP and port can join, up to `--max-players`. Share the
  address only with friends.
