#!/usr/bin/env bash
# Launches a local multiplayer test: one dedicated server plus N game clients (default 2).
#
#   scripts/local_test.sh [options]
#
# Options:
#   --release               build and run optimised binaries
#   --port <PORT>           server UDP port (default 47420)
#   --level <NAME>          level to load (default proving_ground)
#   --clients <N>           number of clients (default 2)
#   --netsim <L,J,LOSS>     network simulator on every client: one-way latency ms, jitter ms,
#                           loss % (e.g. 100,20,2 = 200 ms ping, +-20 ms jitter, 2% loss)
#   --server-netsim <...>   network simulator on the server (applies to all clients)
#   --no-clients            only start the server
#
# Game client flags (binary `velocide`, see docs/network.md):
#   --connect <ip:port>     join a server
#   --host [port]           host a listen server and play on it
#   --name <name>           player name
#   --netsim <l,j,loss>     simulate a bad network on this client
#   (no flags)              single-player: in-process server on loopback
#
# Ctrl-C stops everything. Logs go to the terminal (RUST_LOG=debug for more).

set -euo pipefail
cd "$(dirname "$0")/.."

PROFILE=debug
CARGO_FLAGS=()
PORT=47420
LEVEL=proving_ground
CLIENTS=2
NETSIM=""
SERVER_NETSIM=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --release) PROFILE=release; CARGO_FLAGS+=(--release) ;;
        --port) PORT="$2"; shift ;;
        --level) LEVEL="$2"; shift ;;
        --clients) CLIENTS="$2"; shift ;;
        --no-clients) CLIENTS=0 ;;
        --netsim) NETSIM="$2"; shift ;;
        --server-netsim) SERVER_NETSIM="$2"; shift ;;
        -h|--help) sed -n '2,24p' "$0"; exit 0 ;;
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
[[ -n "$SERVER_NETSIM" ]] && server_args+=(--netsim "$SERVER_NETSIM")
echo "== server: velocide-server ${server_args[*]}"
"$BIN/velocide-server" "${server_args[@]}" &
pids+=($!)
sleep 0.5

for i in $(seq 1 "$CLIENTS"); do
    client_args=(--connect "127.0.0.1:$PORT" --name "P$i")
    [[ -n "$NETSIM" ]] && client_args+=(--netsim "$NETSIM")
    echo "== client $i: velocide ${client_args[*]}"
    "$BIN/velocide" "${client_args[@]}" &
    pids+=($!)
done

echo "== running (server on udp/$PORT). Ctrl-C to stop."
wait "${pids[0]}"
