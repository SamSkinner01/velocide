#!/usr/bin/env bash
# Builds the game and the dedicated server for one platform, as the standalone files a release
# carries (docs/distribution.md), in the current environment: a machine prepared by
# scripts/ci/setup.sh (scripts/package.sh runs this in a local container, the release workflow
# .github/workflows/release.yml in CI).
#
#   scripts/build_dist.sh linux     dist/velocide-linux-x86_64
#                                   dist/velocide-server-linux-x86_64
#   scripts/build_dist.sh windows   dist/velocide-windows-x86_64.exe
#                                   dist/velocide-server-windows-x86_64.exe
#
# Both also write dist/PLAY.txt (how to play, host and join, with the version and the release
# page filled in). Each binary carries the game's assets inside, so it runs on its own.
#
# The build identity the game reads (docs/distribution.md, "Build identity"), handed to cargo:
#
#   VELOCIDE_VERSION   the version, e.g. build-42 or v1.0.0; default $VERSION, else the git
#                      commit (`git describe --always --dirty`)
#   VELOCIDE_COMMIT    the full 40-hex commit; default `git rev-parse HEAD`
#   VELOCIDE_REPO      owner/name the game checks for updates; default $GITHUB_REPOSITORY when
#                      set, else the game's own default
#
# With a commit and a version these are release builds, which check for updates. Everyone in a
# game, the server included, must run the same version.
#
# CARGO_TARGET_DIR is respected (package.sh keeps one per platform). The Linux binaries are x86_64
# on any machine: on an arm64 one they are cross-compiled against the amd64 system libraries
# setup.sh installed.
set -euo pipefail
cd "$(dirname "$0")/.."

WHAT=${1:-}
TARGET_DIR=${CARGO_TARGET_DIR:-target}
# Client and server are built by separate cargo invocations: built together, cargo would unify
# the engine's features and link the renderer/audio into the headless server too.
CARGO_FLAGS=(--release --locked --config 'profile.release.strip="symbols"')

case "$WHAT" in
linux)
    TARGET=x86_64-unknown-linux-gnu
    if [ "$(uname -m)" != x86_64 ]; then
        export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc
        export CC_x86_64_unknown_linux_gnu=x86_64-linux-gnu-gcc
        export AR_x86_64_unknown_linux_gnu=x86_64-linux-gnu-ar
        # Only the amd64 .pc files (the machine's own would link the wrong architecture).
        export PKG_CONFIG_ALLOW_CROSS=1
        export PKG_CONFIG_LIBDIR_x86_64_unknown_linux_gnu=/usr/lib/x86_64-linux-gnu/pkgconfig:/usr/share/pkgconfig
        # The cross linker's own sysroot is /usr/x86_64-linux-gnu; the multiarch libraries'
        # dependencies (libm, libpthread, ...) live in the system's amd64 directories.
        export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-C link-arg=-Wl,-rpath-link,/usr/lib/x86_64-linux-gnu -C link-arg=-Wl,-rpath-link,/lib/x86_64-linux-gnu"
    fi
    EXE=
    ;;
windows)
    TARGET=x86_64-pc-windows-gnu
    export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
    # LLVM's linker: GNU ld prints a spurious "corrupt .drectve" warning for the import stubs
    # rustc generates for Windows API calls.
    export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS='-C link-arg=-fuse-ld=lld'
    EXE=.exe
    ;;
*)
    echo "usage: $0 linux|windows" >&2
    exit 2
    ;;
esac

VELOCIDE_VERSION=${VELOCIDE_VERSION:-${VERSION:-$(git describe --always --dirty 2>/dev/null || echo dev)}}
VELOCIDE_COMMIT=${VELOCIDE_COMMIT:-$(git rev-parse HEAD 2>/dev/null || true)}
VELOCIDE_REPO=${VELOCIDE_REPO:-${GITHUB_REPOSITORY:-}}
export VELOCIDE_VERSION
if [ -z "$VELOCIDE_COMMIT" ]; then
    echo "warning: no git commit (VELOCIDE_COMMIT unset, no repository): a dev build" >&2
    unset VELOCIDE_COMMIT
elif [[ "$VELOCIDE_COMMIT" =~ ^[0-9a-f]{40}$ ]]; then
    export VELOCIDE_COMMIT
else
    echo "error: VELOCIDE_COMMIT must be the full 40-hex commit, not '$VELOCIDE_COMMIT'" >&2
    exit 1
fi
if [ -n "$VELOCIDE_REPO" ]; then export VELOCIDE_REPO; else unset VELOCIDE_REPO; fi

echo "==> building the $WHAT binaries on $(uname -m)"
echo "    VELOCIDE_VERSION=$VELOCIDE_VERSION"
echo "    VELOCIDE_COMMIT=${VELOCIDE_COMMIT:-(unset)}"
echo "    VELOCIDE_REPO=${VELOCIDE_REPO:-(unset: the default of shared::build_info)}"
cargo build "${CARGO_FLAGS[@]}" --target "$TARGET" -p client --bin velocide
cargo build "${CARGO_FLAGS[@]}" --target "$TARGET" -p server --bin velocide-server

out="$TARGET_DIR/$TARGET/release"
game="dist/velocide-$WHAT-x86_64$EXE"
server="dist/velocide-server-$WHAT-x86_64$EXE"
mkdir -p dist
cp "$out/velocide$EXE" "$game"
cp "$out/velocide-server$EXE" "$server"
chmod 755 "$game" "$server"
play=$(<scripts/PLAY.txt)
play=${play//@VERSION@/"$VELOCIDE_VERSION"}
printf '%s\n' "${play//@REPO@/"${VELOCIDE_REPO:-SamSkinner01/velocide}"}" >dist/PLAY.txt
echo "==> $game"
echo "==> $server"
echo "==> dist/PLAY.txt"
