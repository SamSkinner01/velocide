#!/usr/bin/env bash
# Builds one ready-to-share package in the current environment (a container prepared by
# scripts/ci/setup.sh: scripts/package.sh runs it locally, the Forgejo release workflow in CI):
#
#   scripts/build_dist.sh linux     dist/velocide-<version>-linux-x86_64.tar.gz
#   scripts/build_dist.sh windows   dist/velocide-<version>-windows-x86_64.zip
#
# Each package holds velocide, velocide-server, assets/ and PLAY.txt. <version> is $VERSION,
# else the git commit (`-dirty` with uncommitted changes); everyone, including the server, should
# run the same one. CARGO_TARGET_DIR is respected (package.sh keeps one per platform).
#
# The Linux package is x86_64 on any machine: on an arm64 one it is cross-compiled against the
# amd64 system libraries setup.sh installed.
set -euo pipefail
cd "$(dirname "$0")/.."

WHAT=${1:-}
VERSION=${VERSION:-$(git describe --always --dirty 2>/dev/null || echo dev)}
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

echo "==> building $WHAT binaries ($VERSION) on $(uname -m)"
cargo build "${CARGO_FLAGS[@]}" --target "$TARGET" -p client --bin velocide
cargo build "${CARGO_FLAGS[@]}" --target "$TARGET" -p server --bin velocide-server

out="$TARGET_DIR/$TARGET/release"
name="velocide-$VERSION-$WHAT-x86_64"
mkdir -p dist
rm -rf "dist/$name"
mkdir -p "dist/$name"
cp "$out/velocide$EXE" "$out/velocide-server$EXE" "dist/$name/"
cp -r assets "dist/$name/"
sed "s/@VERSION@/$VERSION/" scripts/PLAY.txt >"dist/$name/PLAY.txt"
if [ "$WHAT" = linux ]; then
    archive="$name.tar.gz"
    tar -C dist -czf "dist/$archive" "$name"
else
    archive="$name.zip"
    (cd dist && rm -f "$archive" && zip -qr "$archive" "$name")
fi
rm -rf "dist/$name"
echo "==> dist/$archive"
