#!/usr/bin/env bash
# Regenerates the screenshots in README.md (docs/images/*.webp): renders them headless with
# three client examples, then converts the chosen frames to lossless WebP at 1280x720.
#
#   scripts/readme_shots.sh [--no-render] [scene=N ...]
#
#   --no-render  reuse the frames already in target/readme_shots/ (to pick other ones)
#   scene=N      take candidate N (1-4) of an in-game scene instead of 1, the best by the
#                example's score. Scenes: fight_drowned_chandlery (also picks the colour-vision
#                pair: the same moment), coop, pvp.
#
# Where the images come from (all real screens of the client, no window opens):
#   client_shots  -> target/readme_shots/client/  main menu, loadout, customise, update prompt
#   lobby_shots   -> target/readme_shots/lobby/   the warm-up: lobby panel, the Vestry
#   readme_shots  -> target/readme_shots/game/    a fight, co-op, PvP, colour vision
# The in-game scenes are played for real (servers, Moulds, bots), so each run differs a
# little. Look at the candidates in target/readme_shots/game/ (*_1.png .. *_4.png) and the
# result in docs/images/, and pick others if needed, e.g.
#   scripts/readme_shots.sh --no-render coop=2 fight_drowned_chandlery=3
#
# Needs: cargo (three release builds), a GPU with Vulkan, Metal or DX12 (the examples render
# headless through wgpu), and ImageMagick (`magick`, or `convert` from ImageMagick 6) built
# with WebP support (`magick -list format | grep -i webp`). Takes about 2 minutes after the
# build. docs/images/ is only touched once every image is there.
set -euo pipefail
cd "$(dirname "$0")/.."

RAW=target/readme_shots
OUT=docs/images

render=1
declare -A pick=(
    [fight_drowned_chandlery]=1
    [coop]=1
    [pvp]=1
)
for arg in "$@"; do
    case "$arg" in
    --no-render) render=0 ;;
    -h | --help)
        sed -n '2,24p' "$0"
        exit 0
        ;;
    *=*)
        scene=${arg%%=*}
        n=${arg#*=}
        if [[ -z ${pick[$scene]+x} ]]; then
            echo "unknown scene '$scene' (scenes: ${!pick[*]})" >&2
            exit 2
        fi
        if [[ ! $n =~ ^[1-4]$ ]]; then
            echo "$scene=$n: the candidate is a number from 1 to 4" >&2
            exit 2
        fi
        pick[$scene]=$n
        ;;
    *)
        echo "unknown argument '$arg' (see $0 --help)" >&2
        exit 2
        ;;
    esac
done

if command -v magick >/dev/null; then
    IM=(magick)
elif command -v convert >/dev/null; then
    IM=(convert)
else
    echo "ImageMagick is needed (magick, or convert from ImageMagick 6)" >&2
    exit 1
fi
if ! "${IM[0]}" -list format 2>/dev/null | grep -qi '^ *webp'; then
    echo "this ImageMagick can't write WebP (no libwebp)" >&2
    exit 1
fi

if ((render)); then
    cargo build -p client --release \
        --example client_shots --example lobby_shots --example readme_shots
    rm -rf "$RAW"
    mkdir -p "$RAW"
    # Run from the repository root: an assets/ folder there overrides the built-in files, so
    # the shots show the working tree's levels, models and tuning.
    target/release/examples/client_shots "$RAW/client"
    target/release/examples/lobby_shots "$RAW/lobby"
    target/release/examples/readme_shots "$RAW/game"
fi

# docs/images/<name> <- target/readme_shots/<source>
images=(
    "fight.webp game/fight_drowned_chandlery_${pick[fight_drowned_chandlery]}.png"
    "colour_vision.webp game/colour_vision_${pick[fight_drowned_chandlery]}.png"
    "coop.webp game/coop_${pick[coop]}.png"
    "pvp.webp game/pvp_${pick[pvp]}.png"
    "main_menu.webp client/menu_root.png"
    "loadout.webp client/menu_loadout.png"
    "customise.webp client/menu_customize.png"
    "update.webp client/menu_update.png"
    # A guest's view: the host's shows this machine's LAN address ("FRIENDS JOIN WITH ...").
    "warmup_lobby.webp lobby/warmup_overlay_guest.png"
    "warmup_vestry.webp lobby/warmup_board.png"
)

missing=0
for entry in "${images[@]}"; do
    src=$RAW/${entry#* }
    if [[ ! -f $src ]]; then
        echo "missing $src (did its example fail, or find no moment worth keeping?)" >&2
        missing=1
    fi
done
if ((missing)); then
    echo "docs/images/ left as it was" >&2
    exit 1
fi

mkdir -p "$OUT"
rm -f "$OUT"/*.webp
for entry in "${images[@]}"; do
    name=${entry%% *}
    src=$RAW/${entry#* }
    # Lossless: the pixel art and the HUD text stay sharp, and the dithered 640x360 view
    # compresses well (about 20-350 KB a shot).
    "${IM[@]}" "$src" -strip -resize '1280x720>' \
        -define webp:lossless=true -define webp:method=6 "$OUT/$name"
    echo "$OUT/$name <- $src"
done
du -ch "$OUT"/*.webp | tail -n 1
