#!/usr/bin/env bash
# Re-fetch the upstream real-broadcast captures from their ORIGINAL sources
# (issue #67 / #1259).
#
# The captures the tests need are already mirrored, with a PROVENANCE.md per
# topic, in the private fixtures repo (`private/fixtures/{ts,mpegh,dash,tsduck}/`,
# the `private/` submodule; the two >50 MB files are Git LFS). The shared
# resolver (`test-fixtures` crate: public `fixtures/` -> `private/fixtures/` ->
# `.test-streams/`) finds them there with no script run. This script exists to
# (a) re-derive/verify a mirrored file from upstream, (b) fetch the one capture
# too large to mirror (`russia-t2mi-scte35`, 4.9 GB), and (c) pin the TSDuck
# `tsduck-test` corpus clone (`tsduck-test`, below). Downloads land in the
# gitignored `.test-streams/` scratch dir, the resolver's last-resort tier.
#
# Usage:  tools/fetch-test-streams.sh [name ...]   (no args = fetch all)
#         tools/fetch-test-streams.sh --list
set -euo pipefail

DEST="$(cd "$(dirname "$0")/.." && pwd)/.test-streams"
mkdir -p "$DEST"

# name | url | sha256-or-"-" | what it unlocks | byte-range-or-"-"
# Add entries here as captures are sourced (see the #67 "wanted" table).
# The optional 5th field is an HTTP byte range (`curl -r`, inclusive,
# "start-end") for fetching a short slice of a large/licensed upstream file
# instead of the whole thing — see the #599 entry below.
MANIFEST=(
  "france-tnt-uhf32|https://tsduck.io/streams/france-dttv/tnt-uhf32-562MHz-2019-01-22.ts|-|full real DVB-T mux: SDT/NIT/EIT/PMT + descriptors at scale|-"
  "astra-sgt|https://tsduck.io/streams/astra-sgt/astra-12604H-2025-05-02.ts|-|Astra 19.2E SGT private table (LCN) — 129 MB|-"
  "russia-t2mi-scte35|https://tsduck.io/streams/russia-t2mi-scte35/20180112_1129_UTC_Russian_mux1_T2MI_3PLPs_incl_SCTE35_in_PLP0.ts|-|T2-MI 3 PLPs, SCTE-35 cues in PLP0 — 4.9 GB (large)|-"
  "hotbird-mhp|https://tsduck.io/streams/hotbird-13.0E/hotbird130E-ts6000-2018-05-11.ts|-|Hot Bird 13E, Italy — DVB-MHP object carousels — 336 MB|-"
  "transformers-eia608-h264|https://samples.ffmpeg.org/ffmpeg-bugs/trac/ticket2885/transformers_EIA608_H264.ts|-|real ATSC A/53 caption SEI (EIA-608) in H.264 — issue #599 (short slice of a ~130 MB licensed movie sample; only the first 15 MB is fetched)|0-14999955"
  "mpegh-cicp01-baseline|https://media.githubusercontent.com/media/Fraunhofer-IIS/mpegh-test-content/HEAD/transport_layer_test_content/CICP_01/ts/single/baseline/32kbps/mpegh_cicp_01_baseline_32kbps_single.ts|11d953a3f3cc97ad7076fbf3a7ba2cc1dd64e3b30335a2cfd647b915b2f0ebbd|Fraunhofer-IIS MPEG-H 3D audio TS (CC BY-NC-ND 4.0, private repo only, never public): PMT stream_type 0x2D + MPEG-H_3dAudio_descriptor|-"
  "mpegh-cicp01-lc|https://media.githubusercontent.com/media/Fraunhofer-IIS/mpegh-test-content/HEAD/transport_layer_test_content/CICP_01/ts/single/lc_baseline_compatible/32kbps/mpegh_cicp_01_lc_baseline_compatible_32kbps_single.ts|c860e86f47fe8e0c124ff27c8494960cea98be3f332b625d3629ce8eec53d380|Fraunhofer-IIS MPEG-H 3D audio TS (CC BY-NC-ND 4.0, private repo only, never public): PMT stream_type 0x2D + MPEG-H_3dAudio_descriptor|-"
  "mpegh-cicp06-lc|https://media.githubusercontent.com/media/Fraunhofer-IIS/mpegh-test-content/HEAD/transport_layer_test_content/CICP_06/ts/single/lc_baseline_compatible/32kbps/mpegh_cicp_06_lc_baseline_compatible_32kbps_single.ts|13cebd067fb16ec4ab9fa493dc8dbc791321183ae88795c75a82771bfc8fb83e|Fraunhofer-IIS MPEG-H 3D audio TS (CC BY-NC-ND 4.0, private repo only, never public): PMT stream_type 0x2D + MPEG-H_3dAudio_descriptor|-"
  "mpegh-cicp13-baseline|https://media.githubusercontent.com/media/Fraunhofer-IIS/mpegh-test-content/HEAD/transport_layer_test_content/CICP_13/ts/single/baseline/32kbps/mpegh_cicp_13_baseline_32kbps_single.ts|1215b83d0c5f0ce68d2331cb1a9fa2b7ded4450a5df1e9cdbcac5a39c293273a|Fraunhofer-IIS MPEG-H 3D audio TS (CC BY-NC-ND 4.0, private repo only, never public): PMT stream_type 0x2D + MPEG-H_3dAudio_descriptor|-"
)

# TSDuck test corpus: a 1.6 GB BSD-2-Clause git repo. NOT mirrored wholesale;
# only the files tests reference are in `private/fixtures/tsduck/`. Clone it at
# this exact commit to run the full-corpus (#[ignore]) scans.
TSDUCK_TEST_URL="https://github.com/tsduck/tsduck-test"
TSDUCK_TEST_SHA="100c8b6301437caebe1f2881389170e77389393b"

list() { printf '%-22s %s\n' "NAME" "UNLOCKS"; for e in "${MANIFEST[@]}"; do IFS='|' read -r n u s d r <<<"$e"; printf '%-22s %s\n' "$n" "$d"; done; }
[ "${1:-}" = "--list" ] && { list; echo "tsduck-test            pinned clone of $TSDUCK_TEST_URL @ $TSDUCK_TEST_SHA"; exit 0; }

want=("$@")
if [ ${#want[@]} -eq 0 ] || [[ " ${want[*]} " == *" tsduck-test "* ]]; then
  td="$DEST/tsduck-test"
  [ -d "$td/.git" ] || git clone --no-checkout "$TSDUCK_TEST_URL" "$td"
  git -C "$td" checkout --quiet "$TSDUCK_TEST_SHA"
  echo "✓ tsduck-test @ $TSDUCK_TEST_SHA"
  [ ${#want[@]} -eq 1 ] && exit 0
fi
for e in "${MANIFEST[@]}"; do
  IFS='|' read -r name url sha unlocks range <<<"$e"
  if [ ${#want[@]} -gt 0 ]; then case " ${want[*]} " in *" $name "*) ;; *) continue;; esac; fi
  out="$DEST/$name.ts"
  if [ -f "$out" ]; then echo "✓ $name (already present)"; continue; fi
  echo "↓ $name  <-  $url"
  if [ "$range" != "-" ]; then
    curl -fL --retry 3 -A "rust-dvb-test-fetch" -r "$range" -o "$out.part" "$url"
  else
    curl -fL --retry 3 -A "rust-dvb-test-fetch" -o "$out.part" "$url"
  fi
  if [ "$sha" != "-" ]; then
    got=$(shasum -a 256 "$out.part" | cut -d' ' -f1)
    [ "$got" = "$sha" ] || { echo "✗ sha256 mismatch for $name ($got)"; rm -f "$out.part"; exit 1; }
  fi
  mv "$out.part" "$out"
  echo "✓ $name ($(du -h "$out" | cut -f1))"
done
echo "Done. Streams in: $DEST"
