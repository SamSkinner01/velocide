#!/usr/bin/env bash
# Prepares Ubuntu to build VELOCIDE: system packages and the Rust toolchain. The release workflow
# (.github/workflows/release.yml) runs it in every job that compiles, and scripts/package.sh runs
# it to build its local container images, so CI and local builds are made the same way.
#
#   scripts/ci/setup.sh linux     Ubuntu 20.04: the Linux x86_64 binaries (glibc 2.31)
#   scripts/ci/setup.sh windows   Ubuntu 24.04: the Windows x86_64 binaries (MinGW)
#   scripts/ci/setup.sh check     Ubuntu 24.04: fmt, clippy and the tests, native
#
# The binaries are always x86_64. On an arm64 machine (a Mac's Docker runs arm64 Linux) the
# Linux build is cross-compiled: the x86_64 cross compiler plus the amd64 builds of the system
# libraries the game links (ALSA, udev, D-Bus) from Ubuntu's amd64 archive (multiarch). MinGW
# cross-compiles to Windows from either architecture. CI runs on x86_64 and never needs the
# cross path; scripts/package.sh on an arm64 machine does.
#
# RUST_VERSION picks the toolchain (default: stable). As root (a container) rustup goes to
# /usr/local; as another user (a GitHub-hosted runner) apt-get runs through sudo and the rustup
# already installed there is used. In a CI job the toolchain is put on the PATH of the job's
# later steps.
set -euo pipefail

WHAT=${1:-}
RUST_VERSION=${RUST_VERSION:-stable}
ARCH=$(dpkg --print-architecture)
SUDO=
[ "$(id -u)" = 0 ] || SUDO=sudo

apt_get() {
    $SUDO env DEBIAN_FRONTEND=noninteractive apt-get "$@"
}

COMMON=(curl ca-certificates git build-essential pkg-config)
# What the game links on Linux (cpal: ALSA, gilrs: udev, D-Bus for the desktop portal).
LINUX_LIBS=(libasound2-dev libudev-dev libdbus-1-dev)
TARGETS=()
COMPONENTS=()

case "$WHAT" in
linux)
    if [ "$ARCH" = amd64 ]; then
        apt_get update
        apt_get install -y --no-install-recommends "${COMMON[@]}" "${LINUX_LIBS[@]}"
    else
        # Ports (arm64) for the machine's own packages, the main archive for amd64 ones.
        # shellcheck source=/dev/null
        . /etc/os-release
        $SUDO sed -i -E "s/^deb (http)/deb [arch=$ARCH] \1/" /etc/apt/sources.list
        for suite in "$VERSION_CODENAME" "$VERSION_CODENAME-updates" "$VERSION_CODENAME-security"; do
            echo "deb [arch=amd64] http://archive.ubuntu.com/ubuntu $suite main universe"
        done | $SUDO tee /etc/apt/sources.list.d/amd64.list >/dev/null
        $SUDO dpkg --add-architecture amd64
        apt_get update
        apt_get install -y --no-install-recommends "${COMMON[@]}" gcc-x86-64-linux-gnu \
            libc6-dev-amd64-cross "${LINUX_LIBS[@]/%/:amd64}"
    fi
    TARGETS=(x86_64-unknown-linux-gnu)
    ;;
windows)
    apt_get update
    apt_get install -y --no-install-recommends "${COMMON[@]}" gcc-mingw-w64-x86-64 lld
    TARGETS=(x86_64-pc-windows-gnu)
    ;;
check)
    apt_get update
    apt_get install -y --no-install-recommends "${COMMON[@]}" "${LINUX_LIBS[@]}"
    COMPONENTS=(clippy rustfmt)
    ;;
*)
    echo "usage: $0 linux|windows|check" >&2
    exit 2
    ;;
esac
$SUDO rm -rf /var/lib/apt/lists/*

if [ -z "$SUDO" ]; then
    export RUSTUP_HOME=${RUSTUP_HOME:-/usr/local/rustup} CARGO_HOME=${CARGO_HOME:-/usr/local/cargo}
else
    export RUSTUP_HOME=${RUSTUP_HOME:-$HOME/.rustup} CARGO_HOME=${CARGO_HOME:-$HOME/.cargo}
fi
export PATH=$CARGO_HOME/bin:$PATH
if ! command -v rustup >/dev/null; then
    curl -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path --profile minimal \
        --default-toolchain none
fi
install=(rustup toolchain install "$RUST_VERSION" --profile minimal --no-self-update)
for target in "${TARGETS[@]}"; do install+=(--target "$target"); done
for component in "${COMPONENTS[@]}"; do install+=(--component "$component"); done
"${install[@]}"
rustup default "$RUST_VERSION"
rustc --version

# In a CI job, the toolchain for the job's later steps.
if [ -n "${GITHUB_ENV:-}" ]; then
    echo "RUSTUP_HOME=$RUSTUP_HOME" >>"$GITHUB_ENV"
    echo "CARGO_HOME=$CARGO_HOME" >>"$GITHUB_ENV"
    echo "$CARGO_HOME/bin" >>"$GITHUB_PATH"
fi
