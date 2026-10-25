#!/usr/bin/env bash
# Launches a local multiplayer test: one dedicated server plus N game clients (default 2).
#
#   scripts/local_test.sh [options] [-- extra client args]
#
# Options:
#   --release               build and run optimised binaries
#   --port <PORT>           server UDP port (default 47420)
#   --level <NAME>          level to load (default proving_ground)
#   --mode <MODE>           campaign | deathmatch (default: the level's kind)
#   --lobby                 stay in the lobby: no autostart, clients don't ready up (the
#                           first client is the host and starts the match)
#   --clients <N>           number of clients (default 2)
#   --netsim <L,J,LOSS>     network simulator on every client: one-way latency ms, jitter ms,
#                           loss % (e.g. 100,20,2 = 200 ms ping, +-20 ms jitter, 2% loss)
#   --server-netsim <...>   network simulator on the server (applies to all clients)
#   --no-clients            only start the server
#   --bot                   clients play by themselves (auto-pilot; doesn't grab the mouse)
#   --size <WxH>            client window size (default 960x540, so two fit side by side)
#   --quit-after <SECONDS>  stop everything after this long
#   --capture-dir <DIR>     each client saves DIR/client<N>.png ...
#   --capture-after <SECS>  ... after this many seconds (default 10)
#   --no-audio              clients don't open the audio device
#   --debug                 clients start with the debug overlay (F3) open
#
# The server starts in its lobby (docs/lobby.md). By default it runs with --autostart and the
# clients with --ready, so they gather in the lobby, ready up and drop into the match after
# the 3 s countdown (about 5 s after launch).
#
# Game client flags (binary `velocide`, see docs/client.md):
#   --connect <ip:port>  --host [port]  --sp [level]  --name <name>  --netsim <l,j,loss>
#   --bot  --ready  --autostart  --mode <m>  --capture <png> --capture-after <s>
#   --quit-after <s>  --windowed WxH  --fullscreen     (no flags)  main menu
#
# Example (the Milestone 4 online test, two bots at 200 ms ping, screenshots after 15 s):
#   scripts/local_test.sh --release --level movement_test --netsim 100,20,2 --bot \
#       --quit-after 20 --capture-dir captures/mp --capture-after 15
#
# Ctrl-C stops everything. Logs go to the terminal (RUST_LOG=debug for more).

set -euo pipefail
cd "$(dirname "$0")/.."

PROFILE=debug
CARGO_FLAGS=()
PORT=47420
LEVEL=proving_ground
MODE=""
LOBBY=0
CLIENTS=2
NETSIM=""
SERVER_NETSIM=""
BOT=0
SIZE=960x540
QUIT_AFTER=""
CAPTURE_DIR=""
CAPTURE_AFTER=10
NO_AUDIO=0
EXTRA_DEBUG=0
EXTRA=()

while [[ $# -gt 0 ]]; do
    case "$1" in
        --release) PROFILE=release; CARGO_FLAGS+=(--release) ;;
        --port) PORT="$2"; shift ;;
        --level) LEVEL="$2"; shift ;;
        --mode) MODE="$2"; shift ;;
        --lobby) LOBBY=1 ;;
        --clients) CLIENTS="$2"; shift ;;
        --no-clients) CLIENTS=0 ;;
        --netsim) NETSIM="$2"; shift ;;
        --server-netsim) SERVER_NETSIM="$2"; shift ;;
        --bot) BOT=1 ;;
        --size) SIZE="$2"; shift ;;
        --quit-after) QUIT_AFTER="$2"; shift ;;
        --capture-dir) CAPTURE_DIR="$2"; shift ;;
        --capture-after) CAPTURE_AFTER="$2"; shift ;;
        --no-audio) NO_AUDIO=1 ;;
        --debug) EXTRA_DEBUG=1 ;;
        --) shift; EXTRA=("$@"); break ;;
        -h|--help) sed -n '2,40p' "$0"; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done

export RUST_LOG="${RUST_LOG:-info}"

echo "== building ($PROFILE)"
cargo build "${CARGO_FLAGS[@]}" -p server -p client
BIN="target/$PROFILE"

pids=()
cleanup() {
    trap - EXIT INT TERM
    echo
    echo "== stopping"
    for pid in "${pids[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
    wait 2>/dev/null || true
}
trap cleanup EXIT INT TERM

server_args=(--port "$PORT" --level "$LEVEL")
[[ -n "$MODE" ]] && server_args+=(--mode "$MODE")
[[ "$LOBBY" == 0 ]] && server_args+=(--autostart)
[[ -n "$SERVER_NETSIM" ]] && server_args+=(--netsim "$SERVER_NETSIM")
echo "== server: velocide-server ${server_args[*]}"
"$BIN/velocide-server" "${server_args[@]}" &
pids+=($!)
sleep 0.5

[[ -n "$CAPTURE_DIR" ]] && mkdir -p "$CAPTURE_DIR"
client_pids=()
for i in $(seq 1 "$CLIENTS"); do
    client_args=(--connect "127.0.0.1:$PORT" --name "P$i" --windowed "$SIZE")
    [[ -n "$NETSIM" ]] && client_args+=(--netsim "$NETSIM")
    [[ "$BOT" == 1 ]] && client_args+=(--bot)
    [[ "$LOBBY" == 0 ]] && client_args+=(--ready)
    [[ "$NO_AUDIO" == 1 ]] && client_args+=(--no-audio)
    [[ "$EXTRA_DEBUG" == 1 ]] && client_args+=(--debug)
    [[ -n "$QUIT_AFTER" ]] && client_args+=(--quit-after "$QUIT_AFTER")
    if [[ -n "$CAPTURE_DIR" ]]; then
        client_args+=(--capture "$CAPTURE_DIR/client$i.png" --capture-after "$CAPTURE_AFTER")
    fi
    client_args+=("${EXTRA[@]}")
    echo "== client $i: velocide ${client_args[*]}"
    # Each client gets its own settings file so test runs never touch the player's settings.
    VELOCIDE_CONFIG="${TMPDIR:-/tmp}/velocide-local-test-p$i.ron" \
        "$BIN/velocide" "${client_args[@]}" &
    pids+=($!)
    client_pids+=($!)
    sleep 0.3
done

if [[ -n "$QUIT_AFTER" && ${#client_pids[@]} -gt 0 ]]; then
    echo "== running for ${QUIT_AFTER}s (server on udp/$PORT)"
    wait "${client_pids[@]}" 2>/dev/null || true
else
    echo "== running (server on udp/$PORT). Ctrl-C to stop."
    wait "${pids[0]}"
fi
