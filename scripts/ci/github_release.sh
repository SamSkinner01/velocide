#!/usr/bin/env bash
# GitHub releases for the release workflow (.github/workflows/release.yml), with the gh CLI.
# Needs GH_TOKEN (the run's token, with `contents: write`) and the GITHUB_* variables a run
# sets (repository, commit, server).
#
#   publish <tag> <dir>     release <tag> at the commit $GITHUB_SHA with the files in <dir>,
#                           which must be exactly the assets of docs/distribution.md, marked
#                           latest. A re-run replaces the assets of the release it made before.
#   prune <prefix> <keep>   delete all but the newest <keep> releases tagged <prefix><n>, and
#                           their tags
#   notes <tag>             print the release notes (to read them before a release; outside a
#                           run it uses this checkout's commit and SamSkinner01/velocide)
#
# `gh release create` uploads the files to a draft and publishes it only when every one is up
# (and deletes the draft if an upload fails), so friends never see a half-filled release.
set -euo pipefail
cd "$(dirname "$0")/../.."

REPO=${GITHUB_REPOSITORY:-SamSkinner01/velocide}
SERVER=${GITHUB_SERVER_URL:-https://github.com}
SHA=${GITHUB_SHA:-$(git rev-parse HEAD)}
export GH_REPO=${GH_REPO:-$REPO}

# The release assets (docs/distribution.md).
ASSETS=(
    velocide-windows-x86_64.exe
    velocide-linux-x86_64
    velocide-server-windows-x86_64.exe
    velocide-server-linux-x86_64
    velocide-rendezvous-linux-x86_64
    PLAY.txt
)

title() {
    case "$1" in
    build-*) echo "VELOCIDE build ${1#build-} (${SHA:0:7})" ;;
    *) echo "VELOCIDE $1" ;;
    esac
}

# The release notes, for friends who just want to play.
notes() {
    local tag=$1 page="$SERVER/$REPO"
    local here="$page/releases/download/$tag" latest="$page/releases/latest/download"
    cat <<EOF
Built from [\`${SHA:0:7}\`]($page/commit/$SHA): $(git log -1 --format=%s "$SHA")

**Windows:** download [velocide-windows-x86_64.exe]($here/velocide-windows-x86_64.exe) and double-click it. If Windows says "Windows protected your PC", click **More info**, then **Run anyway**. It asks because the game isn't code-signed.

**Linux:** download [velocide-linux-x86_64]($here/velocide-linux-x86_64), make it executable once with \`chmod +x velocide-linux-x86_64\` (or in the file manager: Properties → Permissions → "Allow executing file as program"), then run it. It needs a Vulkan driver and a distro from about 2020 or newer (glibc 2.31).

The game is that one file: there is nothing to unpack or install, and it runs from any folder. If you played an older version from a zip, put this file in a new folder, not the old one: an old \`assets\` folder next to the game would override what's built into it.

**Everyone in a game needs the same version**, the server too. This one is \`$tag\`. The game checks for a newer version each time it starts and offers to update itself from the main menu, so you only download it by hand once.

These links always fetch the newest version, to bookmark or pass on:

- Windows: $latest/velocide-windows-x86_64.exe
- Linux: $latest/velocide-linux-x86_64

**Most people only need the game.** The \`velocide-server-…\` files are the dedicated server, for running games on a machine nobody plays on ([hosting guide]($page/blob/$SHA/docs/hosting.md)), and \`velocide-rendezvous-linux-x86_64\` is the online service behind join codes and the server browser ([docs/rendezvous.md]($page/blob/$SHA/docs/rendezvous.md)). \`PLAY.txt\` has the controls and how to host and join.
EOF
}

publish() {
    local tag=$1 dir=$2 name files=()
    for name in "${ASSETS[@]}"; do
        [ -f "$dir/$name" ] || { echo "error: $dir/$name is missing" >&2; exit 1; }
        files+=("$dir/$name")
    done
    if [ "$(find "$dir" -type f | wc -l)" -ne "${#ASSETS[@]}" ]; then
        echo "error: $dir holds more than the release assets:" >&2
        find "$dir" -type f >&2
        exit 1
    fi

    local notes_file
    notes_file=$(mktemp)
    notes "$tag" >"$notes_file"
    if gh release view "$tag" >/dev/null 2>&1; then
        echo "release $tag exists already (a re-run): replacing its assets"
        gh release upload "$tag" "${files[@]}" --clobber
        gh release edit "$tag" --target "$SHA" --title "$(title "$tag")" --notes-file "$notes_file" \
            --draft=false --latest
    else
        gh release create "$tag" "${files[@]}" --target "$SHA" --title "$(title "$tag")" \
            --notes-file "$notes_file" --latest
    fi
    rm -f "$notes_file"
    gh release view "$tag" --json url,tagName,targetCommitish,assets \
        --jq '"published \(.tagName) at \(.targetCommitish): \(.url)", (.assets[] | "  \(.name) \(.size) bytes")'
}

prune() {
    local prefix=$1 keep=$2 n
    gh release list --exclude-drafts --limit 1000 --json tagName --jq '.[].tagName' |
        awk -v p="$prefix" 'index($0, p) == 1 && substr($0, length(p) + 1) ~ /^[0-9]+$/ { print substr($0, length(p) + 1) }' |
        sort -rn | tail -n +$((keep + 1)) | while read -r n; do
        # Another run may be pruning at the same time (a tag's run next to a push's).
        if gh release delete "$prefix$n" --cleanup-tag --yes; then
            echo "pruned $prefix$n"
        else
            echo "warning: could not delete $prefix$n (already gone?)" >&2
        fi
    done
}

cmd=${1:-}
shift || true
case "$cmd" in
publish) publish "$@" ;;
prune) prune "$@" ;;
notes) notes "$@" ;;
*)
    echo "usage: $0 publish <tag> <dir> | prune <prefix> <keep> | notes <tag>" >&2
    exit 2
    ;;
esac
