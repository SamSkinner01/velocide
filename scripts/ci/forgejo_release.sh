#!/usr/bin/env bash
# Forgejo releases through the API, for the release workflow (.forgejo/workflows/release.yml).
# Needs curl and jq, and in the environment:
#
#   FORGEJO_API    the instance's API root, e.g. http://192.168.0.100:3000/api/v1
#   FORGEJO_REPO   owner/name
#   FORGEJO_TOKEN  a token that may write the repository (the workflow's automatic token)
#
# Commands:
#
#   draft <tag> <title> <commit> <notes-file>   create a draft release (or reuse the draft
#                                               already made for <tag>); prints its id. The tag is
#                                               created at <commit> when it is published.
#   upload <id> <file>...                       attach files (replacing same-named ones)
#   publish <id>                                publish a draft
#   discard <id>                                delete a draft (a failed run)
#   prune <prefix> <keep>                       delete all but the newest <keep> releases whose
#                                               tag starts with <prefix>, and their tags
#
# A build's assets go up to a draft first and the release is published only when every package
# built, so a failed run never leaves a half-filled release behind.
set -euo pipefail

: "${FORGEJO_API:?}" "${FORGEJO_REPO:?}" "${FORGEJO_TOKEN:?}"
BASE="$FORGEJO_API/repos/$FORGEJO_REPO"

# api <method> <path> [curl args...]: the response body on 2xx, else the error and exit 1.
api() {
    local method=$1 path=$2 out status
    shift 2
    out=$(mktemp)
    status=$(curl -sS -o "$out" -w '%{http_code}' -X "$method" \
        -H "Authorization: token $FORGEJO_TOKEN" -H 'Accept: application/json' \
        "$@" "$BASE$path")
    if [ "${status:0:1}" != 2 ]; then
        echo "error: $method $path -> HTTP $status: $(cat "$out")" >&2
        rm -f "$out"
        return 1
    fi
    cat "$out"
    rm -f "$out"
}

# Every release, newest first (drafts too: the token may write the repository; a `draft` query
# parameter would list only drafts).
all_releases() {
    local page=1 batch
    while :; do
        batch=$(api GET "/releases?limit=50&page=$page")
        [ "$(jq length <<<"$batch")" -gt 0 ] || break
        jq -c '.[]' <<<"$batch"
        page=$((page + 1))
    done | jq -sc 'sort_by(-.id) | .[]'
}

cmd=${1:-}
shift || true
case "$cmd" in
draft)
    tag=$1 title=$2 commit=$3 notes=$4
    id=$(all_releases | jq -r --arg tag "$tag" 'select(.tag_name == $tag and .draft) | .id' | head -n1)
    if [ -z "$id" ]; then
        id=$(jq -n --arg tag "$tag" --arg title "$title" --arg commit "$commit" --rawfile body "$notes" \
            '{tag_name: $tag, name: $title, target_commitish: $commit, body: $body, draft: true, prerelease: false}' |
            api POST /releases -H 'Content-Type: application/json' --data-binary @- | jq -r .id)
    fi
    echo "$id"
    ;;
upload)
    id=$1
    shift
    for file in "$@"; do
        name=$(basename "$file")
        api GET "/releases/$id/assets" | jq -r --arg name "$name" '.[] | select(.name == $name) | .id' |
            while read -r old; do api DELETE "/releases/$id/assets/$old" >/dev/null; done
        api POST "/releases/$id/assets?name=$name" -F "attachment=@$file" >/dev/null
        echo "uploaded $name"
    done
    ;;
publish)
    jq -n '{draft: false}' | api PATCH "/releases/$1" -H 'Content-Type: application/json' --data-binary @- |
        jq -r '"published \(.tag_name): \(.html_url)"'
    ;;
discard)
    if api GET "/releases/$1" | jq -e .draft >/dev/null; then
        api DELETE "/releases/$1" >/dev/null
        echo "discarded draft $1"
    fi
    ;;
prune)
    prefix=$1 keep=$2
    all_releases | jq -r --arg p "$prefix" 'select((.tag_name | startswith($p)) and (.draft | not)) | "\(.id) \(.tag_name)"' |
        tail -n +$((keep + 1)) | while read -r id tag; do
        api DELETE "/releases/$id" >/dev/null
        api DELETE "/tags/$tag" >/dev/null || true
        echo "pruned $tag"
    done
    ;;
*)
    echo "usage: $0 draft|upload|publish|discard|prune ..." >&2
    exit 2
    ;;
esac
