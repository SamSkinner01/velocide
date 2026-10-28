#!/usr/bin/env bash
# Builds the files to share with friends, the same ones a release carries (docs/distribution.md):
#
#   dist/velocide-linux-x86_64                 the game, standalone
#   dist/velocide-server-linux-x86_64          the dedicated server, standalone
#   dist/velocide-rendezvous-linux-x86_64      the online service (docs/rendezvous.md)
#   dist/velocide-windows-x86_64.exe           the game, standalone
#   dist/velocide-server-windows-x86_64.exe    the dedicated server, standalone
#   dist/PLAY.txt                              how to play, host and join
#
#   scripts/package.sh [linux|windows|all]     (default: all)
#
# Builds run inside containers (podman, or docker) so the result works on other people's
# machines:
# - Linux is built on Ubuntu 20.04, so the binaries need only glibc 2.31 (any distro from
#   about 2020 on). A plain `cargo build` on a new distro would need that distro's newer glibc.
# - Windows is cross-compiled with MinGW (from Ubuntu 24.04, for a recent MinGW linker).
# The containers are prepared by scripts/ci/setup.sh and build with scripts/build_dist.sh, the
# same scripts the release workflow runs (.github/workflows/release.yml), with the same Rust
# version as this machine; the host's crate cache is mounted so dependencies aren't downloaded
# again. The first run builds two images; later runs only compile.
#
# The version is the git commit (`-dirty` if there are uncommitted changes), or
# $VELOCIDE_VERSION. Everyone, including the server, should run the same version. The commit
# is built in too, so these are release builds: they check GitHub for a newer release.
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
# The build identity is taken here: git inside the container would refuse the mounted
# repository (it belongs to another user there, "dubious ownership").
IDENTITY=(
    -e VELOCIDE_VERSION="${VELOCIDE_VERSION:-$(git describe --always --dirty 2>/dev/null || echo dev)}"
    -e VELOCIDE_COMMIT="${VELOCIDE_COMMIT:-$(git rev-parse HEAD 2>/dev/null || true)}"
)
[ -z "${VELOCIDE_REPO:-}" ] || IDENTITY+=(-e VELOCIDE_REPO="$VELOCIDE_REPO")
[ -z "${VELOCIDE_RENDEZVOUS:-}" ] || IDENTITY+=(-e VELOCIDE_RENDEZVOUS="$VELOCIDE_RENDEZVOUS")
REGISTRY="${CARGO_HOME:-$HOME/.cargo}/registry"
mkdir -p dist "$REGISTRY"

# Builds a toolchain image with scripts/ci/setup.sh (once per Rust version and setup script).
# $1 = image name, $2 = base image, $3 = setup.sh's argument.
build_image() {
    local image=$1 base=$2 kind=$3
    # The tag includes the recipe, so changing the setup script builds a fresh image.
    image="$image-$(md5sum <scripts/ci/setup.sh | cut -c1-8)"
    IMAGE=$image
    if "$ENGINE" image exists "$image" 2>/dev/null || "$ENGINE" image inspect "$image" >/dev/null 2>&1; then
        return
    fi
    echo "==> building container image $image (one-time)"
    "$ENGINE" build -t "$image" -f - scripts/ci <<EOF
FROM docker.io/library/$base
COPY setup.sh /tmp/setup.sh
RUN RUST_VERSION=$RUST_VERSION bash /tmp/setup.sh $kind
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:\$PATH
EOF
}

# Builds one platform in a container with the repo mounted at /src. $1 = image, $2 = target
# directory under target/, $3 = linux | windows.
run_build() {
    local image=$1 target_dir=$2 what=$3
    "$ENGINE" run --rm --security-opt label=disable \
        -v "$ROOT":/src -v "$REGISTRY":/usr/local/cargo/registry \
        -e CARGO_TARGET_DIR="/src/target/$target_dir" "${IDENTITY[@]}" -w /src \
        "$image" bash scripts/build_dist.sh "$what"
}

if [ "$WHAT" = linux ] || [ "$WHAT" = all ]; then
    build_image "velocide-build-linux:$RUST_VERSION" ubuntu:20.04 linux
    run_build "$IMAGE" dist-linux linux
fi

if [ "$WHAT" = windows ] || [ "$WHAT" = all ]; then
    build_image "velocide-build-windows:$RUST_VERSION" ubuntu:24.04 windows
    run_build "$IMAGE" dist-windows windows
fi

ls -l dist
