#!/usr/bin/env bash
# Builds ready-to-share game packages for friends:
#
#   dist/velocide-<version>-linux-x86_64.tar.gz    velocide, velocide-server, assets/, PLAY.txt
#   dist/velocide-<version>-windows-x86_64.zip     velocide.exe, velocide-server.exe, assets\, PLAY.txt
#
#   scripts/package.sh [linux|windows|all]          (default: all)
#
# Builds run inside containers (podman, or docker) so the result works on other people's
# machines:
# - Linux is built on Ubuntu 20.04, so the binaries need only glibc 2.31 (any distro from
#   about 2020 on). A plain `cargo build` on a new distro would need that distro's newer glibc.
# - Windows is cross-compiled with MinGW (from Ubuntu 24.04, for a recent MinGW linker).
# The containers use the same Rust version as this machine, and the host's crate cache is
# mounted so dependencies aren't downloaded again. The first run builds two images; later runs
# only compile.
#
# <version> is the git commit (`-dirty` if there are uncommitted changes). Everyone, including
# the server, should run the same version.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)

WHAT=${1:-all}
case "$WHAT" in linux | windows | all) ;; *)
    echo "usage: $0 [linux|windows|all]" >&2
    exit 2
    ;;
esac

ENGINE=$(command -v podman || command -v docker || true)
if [ -z "$ENGINE" ]; then
    echo "error: podman or docker is required" >&2
    exit 1
fi
RUST_VERSION=$(rustc --version | awk '{print $2}')
VERSION=$(git describe --always --dirty 2>/dev/null || echo dev)
REGISTRY="${CARGO_HOME:-$HOME/.cargo}/registry"
mkdir -p dist "$REGISTRY"

# Builds a toolchain image (once per Rust version). $1 = image name, $2 = base image, $3 = extra
# apt packages, $4 = extra rustup target.
build_image() {
    local image=$1 base=$2 packages=$3 target=$4
    # Tag includes the recipe, so changing the package list builds a fresh image.
    image="$image-$(echo "$base $packages $target" | md5sum | cut -c1-8)"
    IMAGE=$image
    if "$ENGINE" image exists "$image" 2>/dev/null || "$ENGINE" image inspect "$image" >/dev/null 2>&1; then
        return
    fi
    echo "==> building container image $image (one-time)"
    "$ENGINE" build -t "$image" -f - <<EOF
FROM docker.io/library/$base
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
        curl ca-certificates build-essential pkg-config $packages \
    && rm -rf /var/lib/apt/lists/*
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:\$PATH
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain $RUST_VERSION \
    && ${target:+rustup target add $target &&} rustc --version
EOF
}

# Runs a build command in a container with the repo mounted at /src.
run_build() {
    local image=$1 target_dir=$2 command=$3
    "$ENGINE" run --rm --security-opt label=disable \
        -v "$ROOT":/src -v "$REGISTRY":/usr/local/cargo/registry \
        -e CARGO_TARGET_DIR="/src/target/$target_dir" -w /src \
        "$image" bash -c "$command"
}

CARGO_BUILD='cargo build --release --locked --config "profile.release.strip=\"symbols\"" -p client -p server --bin velocide --bin velocide-server'

# Copies binaries + assets + PLAY.txt into a fresh staging folder. $1 = folder, $2.. = binaries.
stage() {
    local dir=$1
    shift
    rm -rf "$dir"
    mkdir -p "$dir"
    cp "$@" "$dir/"
    cp -r assets "$dir/"
    sed "s/@VERSION@/$VERSION/" scripts/PLAY.txt >"$dir/PLAY.txt"
}

if [ "$WHAT" = linux ] || [ "$WHAT" = all ]; then
    build_image "velocide-build-linux:$RUST_VERSION" ubuntu:20.04 "libasound2-dev libudev-dev libdbus-1-dev" ""
    echo "==> building Linux binaries"
    run_build "$IMAGE" dist-linux "$CARGO_BUILD"
    out="target/dist-linux/release"
    name="velocide-$VERSION-linux-x86_64"
    stage "dist/$name" "$out/velocide" "$out/velocide-server"
    tar -C dist -czf "dist/$name.tar.gz" "$name"
    rm -rf "dist/$name"
    echo "==> dist/$name.tar.gz"
fi

if [ "$WHAT" = windows ] || [ "$WHAT" = all ]; then
    build_image "velocide-build-windows:$RUST_VERSION" ubuntu:24.04 "gcc-mingw-w64-x86-64 lld" "x86_64-pc-windows-gnu"
    echo "==> building Windows binaries"
    # LLVM's linker: GNU ld prints a spurious "corrupt .drectve" warning for the import
    # stubs rustc generates for Windows API calls.
    run_build "$IMAGE" dist-windows \
        "CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS='-C link-arg=-fuse-ld=lld' $CARGO_BUILD --target x86_64-pc-windows-gnu"
    out="target/dist-windows/x86_64-pc-windows-gnu/release"
    name="velocide-$VERSION-windows-x86_64"
    stage "dist/$name" "$out/velocide.exe" "$out/velocide-server.exe"
    (cd dist && rm -f "$name.zip" && zip -qr "$name.zip" "$name")
    rm -rf "dist/$name"
    echo "==> dist/$name.zip"
fi
