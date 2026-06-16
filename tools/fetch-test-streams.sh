#!/usr/bin/env bash
# Fetch large real-broadcast capture fixtures ON DEMAND (issue #67).
#
# These captures are NOT checked into the repo (too large; they are someone
# else's published test streams). This script pulls them into the gitignored
# `.test-streams/` dir; the `downloaded_streams` integration tests pick up
# whatever is present and SKIP cleanly when the dir is empty. CI can run this
# to enable the extended-capture tests; locally it's opt-in.
#
# Usage:  tools/fetch-test-streams.sh [name ...]   (no args = fetch all)
#         tools/fetch-test-streams.sh --list
set -euo pipefail

DEST="$(cd "$(dirname "$0")/.." && pwd)/.test-streams"
mkdir -p "$DEST"

# name | url | sha256-or-"-" | what it unlocks
# Add entries here as captures are sourced (see the #67 "wanted" table).
MANIFEST=(
  "france-tnt-uhf32|https://tsduck.io/streams/france-dttv/tnt-uhf32-562MHz-2019-01-22.ts|-|full real DVB-T mux: SDT/NIT/EIT/PMT + descriptors at scale"
  "astra-sgt|https://tsduck.io/streams/astra-sgt/astra-12604H-2025-05-02.ts|-|Astra 19.2E SGT private table (LCN) — 129 MB"
  "russia-t2mi-scte35|https://tsduck.io/streams/russia-t2mi-scte35/20180112_1129_UTC_Russian_mux1_T2MI_3PLPs_incl_SCTE35_in_PLP0.ts|-|T2-MI 3 PLPs, SCTE-35 cues in PLP0 — 4.9 GB (large)"
  "hotbird-mhp|https://tsduck.io/streams/hotbird-13.0E/hotbird130E-ts6000-2018-05-11.ts|-|Hot Bird 13E, Italy — DVB-MHP object carousels — 336 MB"
)

list() { printf '%-22s %s\n' "NAME" "UNLOCKS"; for e in "${MANIFEST[@]}"; do IFS='|' read -r n u s d <<<"$e"; printf '%-22s %s\n' "$n" "$d"; done; }
[ "${1:-}" = "--list" ] && { list; exit 0; }

want=("$@")
for e in "${MANIFEST[@]}"; do
  IFS='|' read -r name url sha unlocks <<<"$e"
  if [ ${#want[@]} -gt 0 ]; then case " ${want[*]} " in *" $name "*) ;; *) continue;; esac; fi
  out="$DEST/$name.ts"
  if [ -f "$out" ]; then echo "✓ $name (already present)"; continue; fi
  echo "↓ $name  <-  $url"
  curl -fL --retry 3 -A "rust-dvb-test-fetch" -o "$out.part" "$url"
  if [ "$sha" != "-" ]; then
    got=$(shasum -a 256 "$out.part" | cut -d' ' -f1)
    [ "$got" = "$sha" ] || { echo "✗ sha256 mismatch for $name ($got)"; rm -f "$out.part"; exit 1; }
  fi
  mv "$out.part" "$out"
  echo "✓ $name ($(du -h "$out" | cut -f1))"
done
echo "Done. Streams in: $DEST"
