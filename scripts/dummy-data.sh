#!/usr/bin/env bash
# Writes a disposable Publicator data dir (config, publications, art) for UI testing.
# usage: scripts/dummy-data.sh [DIR]   (default: a fresh /tmp/publicator-dummy.XXXXXX)
set -euo pipefail

dir=${1:-$(mktemp -d /tmp/publicator-dummy.XXXXXX)}
mkdir -p "$dir/picked" "$dir/published"
now=$(date +%s)
day=86400

art() { # art FILE SEED WIDTH HEIGHT
  magick -seed "$2" -size "$3x$4" plasma:fractal -blur 0x2 "$1"
}

for i in 1 2 3 4 5 6 7 8; do
  art "$dir/picked/candidate-$i.png" "$i" $((480 + i % 3 * 160)) $((640 - i % 2 * 240))
done

cat >"$dir/publicator.toml" <<'EOF'
publicable = ["picked"]

[deviantart]
tiers = ["Supporter", "Patron"]
galleries = ["Featured", "Landscapes", "Portraits"]

[schedule]
timezone = "Europe/Paris"

[[schedule.profiles]]
name = "Tuesday evening"
day = "tuesday"
hour = 20
per_slot = 2

[[schedule.profiles]]
name = "Saturday morning"
day = "saturday"
hour = 10
per_slot = 1
EOF

marker='[[PUBLICATOR:TODO_GENERATE]]'
rows=()
row() { # row N STATE TITLE DESCRIPTION OFFSET_DAYS APPARITIONS
  local name="art-$1.png"
  art "$dir/published/$name" "$((100 + $1))" 600 $((400 + $1 % 3 * 200))
  rows+=("$(jq -n --arg uuid "$(printf '00000000-0000-4000-8000-%012d' "$1")" \
    --arg state "$2" --arg title "$3" --arg description "$4" --arg name "$name" \
    --arg sha "$(sha512sum "$dir/published/$name" | cut -d' ' -f1)" \
    --argjson submitted "$((now - 30 * day))" --argjson at "$((now + $5 * day))" \
    --argjson apparitions "$6" \
    '{uuid:$uuid, state:$state, title:$title, description:$description,
      submissionTimestamp:$submitted, apparitionTimestampIfDifferentThanSubmission:$at,
      files:[{basename:$name, sha512sum:$sha}], apparitions:$apparitions}')")
}
draft='[{"platformName":"deviantart","state":"unpublished","tiers":["Supporter"],"galleries":["Featured"]}]'
live() { printf '[{"platformName":"deviantart","state":"published_or_scheduled","url":"https://example.test/art/%s"}]' "$1"; }

row 1 APPEARED "Harbor at dusk" "Warm light over the old harbor." -24 "$(live 1)"
row 2 APPEARED "Glass forest" "Refraction study." -17 "$(live 2)"
row 3 APPEARED "Night market" "Neon and rain." -3 "$(live 3)"
row 4 DESCRIBED "Copper dunes" "Desert at noon." 4 "$draft"
row 5 DESCRIBED "Tidal bloom" "Algae under moonlight." 8 "$draft"
row 6 SCHEDULED "Signal tower" "Radio mast in fog." 11 '[]'
row 7 SCHEDULED "$marker" "$marker" 15 '[]'
row 8 SCHEDULED "Paper moons" "" 18 '[]'
row 9 PICKED "" "" 22 '[]'
row 10 PICKED "" "" 25 '[]'

printf '%s\n' "${rows[@]}" | jq -s . >"$dir/publications.json"
echo "$dir"
