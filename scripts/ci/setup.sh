#!/usr/bin/env bash
# Prepares a fresh Ubuntu container to build VELOCIDE: system packages and the Rust toolchain.
# Used by the Forgejo release workflow (.forgejo/workflows/release.yml) in every job, and by
# scripts/package.sh to build its local container images, so both build the same way.
#
#   scripts/ci/setup.sh linux     Ubuntu 20.04: the Linux x86_64 package (glibc 2.31)
#   scripts/ci/setup.sh windows   Ubuntu 24.04: the Windows x86_64 package (MinGW)
#   scripts/ci/setup.sh check     Ubuntu 24.04: fmt, clippy and the tests, native
#
# The packages are always x86_64. On an arm64 machine (a Mac's Docker runs arm64 Linux) the
# Linux build is cross-compiled: the x86_64 cross compiler plus the amd64 builds of the system
# libraries the game links (ALSA, udev, D-Bus) from Ubuntu's amd64 archive (multiarch). MinGW
# cross-compiles to Windows from either architecture.
#
# RUST_VERSION picks the toolchain (default: stable). Run as root (containers are).
set -euo pipefail

WHAT=${1:-}
RUST_VERSION=${RUST_VERSION:-stable}
export DEBIAN_FRONTEND=noninteractive
ARCH=$(dpkg --print-architecture)

COMMON="curl ca-certificates git build-essential pkg-config zip jq"
# What the game links on Linux (cpal: ALSA, gilrs: udev, D-Bus for the desktop portal).
LINUX_LIBS="libasound2-dev libudev-dev libdbus-1-dev"

case "$WHAT" in
linux)
    if [ "$ARCH" = amd64 ]; then
        apt-get update
        apt-get install -y --no-install-recommends $COMMON $LINUX_LIBS
    else
        # Ports (arm64) for the machine's own packages, the main archive for amd64 ones.
        . /etc/os-release
        sed -i -E "s/^deb (http)/deb [arch=$ARCH] \1/" /etc/apt/sources.list
        for suite in "$VERSION_CODENAME" "$VERSION_CODENAME-updates" "$VERSION_CODENAME-security"; do
            echo "deb [arch=amd64] http://archive.ubuntu.com/ubuntu $suite main universe"
        done >/etc/apt/sources.list.d/amd64.list
        dpkg --add-architecture amd64
        apt-get update
        apt-get install -y --no-install-recommends $COMMON gcc-x86-64-linux-gnu libc6-dev-amd64-cross \
            $(for lib in $LINUX_LIBS; do echo "$lib:amd64"; done)
    fi
    TARGETS=x86_64-unknown-linux-gnu
    COMPONENTS=
    ;;
windows)
    apt-get update
    apt-get install -y --no-install-recommends $COMMON gcc-mingw-w64-x86-64 lld
    TARGETS=x86_64-pc-windows-gnu
    COMPONENTS=
    ;;
check)
    apt-get update
    apt-get install -y --no-install-recommends $COMMON $LINUX_LIBS
    TARGETS=
    COMPONENTS="clippy rustfmt"
    ;;
*)
    echo "usage: $0 linux|windows|check" >&2
    exit 2
    ;;
esac
rm -rf /var/lib/apt/lists/*

export RUSTUP_HOME=${RUSTUP_HOME:-/usr/local/rustup} CARGO_HOME=${CARGO_HOME:-/usr/local/cargo}
curl -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path --profile minimal \
    --default-toolchain "$RUST_VERSION"
PATH=$CARGO_HOME/bin:$PATH
[ -z "$TARGETS" ] || rustup target add $TARGETS
[ -z "$COMPONENTS" ] || rustup component add $COMPONENTS
rustc --version
