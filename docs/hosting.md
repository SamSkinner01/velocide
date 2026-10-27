# Hosting a dedicated server (e.g. a DigitalOcean droplet)

The dedicated server is one statically linked binary plus the `assets/` folder. It needs no
GPU, no audio and no Rust toolchain on the machine it runs on.

## 1. Build the server (on your own PC)

**Easiest:** if you already ran `scripts/package.sh linux` to share the game, the
`velocide-server` binary and the `assets/` folder in that package work on the server as is.
They need nothing beyond the C library. Copy the unpacked folder's contents to
`/opt/velocide/` and skip to step 3.

**Or** build just the server as a fully static binary:

```sh
rustup target add x86_64-unknown-linux-musl        # one time
cargo build --release -p server --bin velocide-server --target x86_64-unknown-linux-musl
```

The output is `target/x86_64-unknown-linux-musl/release/velocide-server`, about 6 MB. It is a
static binary, so it runs on any x86-64 Linux, including Ubuntu and Debian droplets.

## 2. Copy it to the server

```sh
ssh root@SERVER_IP 'mkdir -p /opt/velocide'
scp target/x86_64-unknown-linux-musl/release/velocide-server root@SERVER_IP:/opt/velocide/
scp -r assets root@SERVER_IP:/opt/velocide/
```

The server looks for `assets/` next to its binary. You can also point it elsewhere with the
`VELOCIDE_ASSETS` environment variable.

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

Everyone (the server and every player) must run the same version of the game. After pulling or
changing code:

```sh
cargo build --release -p server --bin velocide-server --target x86_64-unknown-linux-musl
ssh root@SERVER_IP 'systemctl stop velocide'
scp target/x86_64-unknown-linux-musl/release/velocide-server root@SERVER_IP:/opt/velocide/
scp -r assets root@SERVER_IP:/opt/velocide/
ssh root@SERVER_IP 'chown -R velocide:velocide /opt/velocide && systemctl start velocide'
```

Stop the service before copying, otherwise the binary is busy and won't be replaced.

Gameplay numbers can be changed live: edit `/opt/velocide/assets/tuning.ron` on the server. It
is reloaded on save and pushed to every connected player.

Or take the server from a CI release (below): the Linux package holds `velocide-server` and
`assets/` already.

```sh
TAG=build-42   # the release to run, e.g. the newest one on the Releases page
curl -LO http://192.168.0.100:3000/sam/velocide/releases/download/$TAG/velocide-$TAG-linux-x86_64.tar.gz
```

## Releases from CI

`.forgejo/workflows/release.yml` builds and publishes the packages on the Forgejo instance, so
nobody has to run `scripts/package.sh` by hand:

| trigger | release |
|---|---|
| a push to `main` | `build-<n>` (`<n>`: the run number), titled with the commit; the newest 10 are kept |
| a `v*` tag (`git tag v1.0 && git push origin v1.0`) | under that tag, kept |
| *Run workflow* on the Actions page | as a push to `main` |

Every release carries `velocide-<version>-linux-x86_64.tar.gz` and
`velocide-<version>-windows-x86_64.zip`, where `<version>` is the release's tag (it is also what
`PLAY.txt` shows: everyone must run the same one). `/sam/velocide/releases/latest` always opens
the newest.

A run has five stages, each a job in a fresh Ubuntu container on the `mac-studio-docker` runner:

1. **check**: `cargo fmt --check`, `cargo clippy -D warnings` and `cargo test --workspace`.
   Nothing is released unless they pass.
2. **draft**: a draft release for the version.
3. **linux** (Ubuntu 20.04, for glibc 2.31) and **windows** (Ubuntu 24.04, MinGW): build their
   package with `scripts/build_dist.sh` and attach it to the draft.
4. **publish**: the draft becomes the release (its tag is created at the built commit), then
   old `build-*` releases are pruned.
5. **discard** (only after a failure): deletes the draft, so a failed run leaves nothing half
   done.

How it is built:

- Every step is a script in the repo (`scripts/ci/setup.sh`: packages + rustup;
  `scripts/build_dist.sh`: build + archive; `scripts/ci/forgejo_release.sh`: the release API),
  so the workflow needs no actions from a registry: the runner only needs Ubuntu's archive,
  rustup and crates.io. `scripts/package.sh` runs the same two build scripts in local
  containers, so a local package and a CI one are built the same way.
- The packages are always x86_64. A Mac's Docker runs arm64 Linux containers, so there the Linux
  package is cross-compiled (`gcc-x86-64-linux-gnu` and the amd64 builds of ALSA, udev and
  D-Bus from Ubuntu's amd64 archive); MinGW cross-compiles to Windows from either.
- The toolchain is pinned by `RUST_VERSION` at the top of the workflow; bump it with the
  machine you develop on.
- The jobs use the run's automatic token (repository write: checkout, releases, tags). If an
  instance restricts it, add a repository secret `RELEASE_TOKEN` (an access token with
  repository write) and the workflow uses that instead.
- Each run compiles from scratch (there's no cache): a few minutes per job on the Mac Studio.
  With runner capacity 2, `linux` and `windows` build at the same time.

## Notes

- **Size:** the smallest droplet is plenty. A server tick with 8 players costs about 0.1 ms, and
  each player uses about 10–30 KB/s. Pick the region closest to your group, since ping matters
  more than CPU.
- **No password:** anyone who knows the IP and port can join, up to `--max-players`. Share the
  address only with friends.
