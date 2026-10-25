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

## Notes

- **Size:** the smallest droplet is plenty. A server tick with 8 players costs about 0.1 ms, and
  each player uses about 10–30 KB/s. Pick the region closest to your group, since ping matters
  more than CPU.
- **No password:** anyone who knows the IP and port can join, up to `--max-players`. Share the
  address only with friends.
