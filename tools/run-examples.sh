#!/usr/bin/env bash
# Run every argument-free workspace example and require it to succeed.
#
# Why this exists (issue #947): `cargo build --workspace --examples` only proves
# an example COMPILES. Eleven examples pointed at fixture paths that stopped
# existing after the fixtures migration, and each swallowed the failure with a
# "fixture not available; nothing to do" fallback — so they exited 0, printed a
# line nobody read, and had never actually run. A build gate cannot catch that.
#
# Rules enforced here:
#   1. An argument-free example must exit 0.
#   2. It must not report that it skipped its own committed fixture. A committed
#      fixture is always present; if it cannot be read, that is a bug.
#
#   3. An example that REQUIRES arguments must declare them in example_args()
#      below, and is then run with them. Undeclared is a failure, not a skip —
#      the original "skip (needs an argument)" branch meant six examples never
#      ran at all.
#
# Usage: tools/run-examples.sh [extra cargo args...]
set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

# Examples that bind a socket or otherwise wait on the network. They are
# long-running servers by design, not self-terminating demos, so running them
# here would hang or collide on a port. Still build-gated.
#
# `webrtc-runtime/whip_media_smoke` joined this list when the MSRV bump
# (#949) removed the exclusion that had always skipped it, so it ran
# unattended for the first time: it binds a FIXED signalling port and then
# blocks waiting for a real external WHIP publisher to connect. Both failure
# modes showed up immediately — a 32-minute hang on the first run, then
# `AddrInUse` on the next because the hung instance still held the port.
SERVER_EXAMPLES=" rtmp-runtime/capture_publish multimux/serve_rtsp webrtc-runtime/whip_media_smoke "

# Crates whose examples cannot be built under this invocation's feature set /
# toolchain. Empty by default.
#
# This list used to hold `webrtc-runtime` and `multimux`, whose optional
# ICE/DTLS-SRTP features needed rustc >= 1.88 while the workspace MSRV was
# 1.86. The MSRV is now 1.95.0 (issue #949), so `--all-features` reaches
# every crate and nothing needs excluding. The mechanism stays because the
# situation recurs; the default is empty because right now it must be.
SKIP_CRATES="${SKIP_CRATES-}"

# When non-empty, run ONLY these crates' examples (same space-delimited form).
# Whitespace-only counts as empty: the lists are written with padding spaces
# (" a b "), so " " is the natural way to spell "none" and must not be read as
# a filter that matches nothing.
ONLY_CRATES="${ONLY_CRATES-}"
[ -z "${ONLY_CRATES// /}" ] && ONLY_CRATES=""

# Scratch dir for examples that write an output file.
WORKDIR=$(mktemp -d)
trap 'rm -rf "$WORKDIR"' EXIT

# Arguments for examples that REQUIRE them. An example listed here runs with
# these arguments; an example that needs arguments and is NOT listed is a
# FAILURE, not a skip.
#
# This closes the other half of issue #947. The original runner let any
# argument-taking example exit non-zero, matched its usage message, and pass
# as "skip (needs an argument)" — so six examples were never executed by any
# gate, which is the same silent hole #947 was opened about, just reached by a
# different route. Every argument-taking example now has a declared, committed
# invocation against a real fixture.
example_args() {
    case "$1" in
        broadcast-bitstream/split_annexb)
            echo "fixtures/candidates/h264.annexb" ;;
        broadcast-bitstream/annexb_to_avcc)
            echo "fixtures/candidates/h264.annexb" ;;
        broadcast-bitstream/decode_capture_sps)
            echo "fixtures/ts/h264-aac.ts" ;;
        broadcast-bitstream/print_capture_slices)
            echo "fixtures/ts/h264-aac.ts" ;;
        broadcast-bitstream/print_adts_headers)
            echo "fixtures/audio/aac.adts" ;;
        broadcast-bitstream/print_mpeg_audio_headers)
            echo "fixtures/audio/audio.mp3" ;;
        isobmff/heif_info) echo "fixtures/heif/thumb.heic" ;;
        isobmff/build_avif) echo "fixtures/avif/plain.avif" ;;
        isobmff/heif_2025) echo "fixtures/heif/groups.mp4" ;;
        atsc3/parse_lls)      echo "fixtures/atsc3/slt-lls-2019-01-07.bin" ;;
        atsc3/parse_slt)      echo "fixtures/atsc3/slt-lls-2019-01-07.bin" ;;
        st2022/parse_hbrmt)   echo "fixtures/rtp/st2022-6-hbrmt-payload-header.bin" ;;
        dvb-mabr/parse_session) echo "fixtures/mabr/annex-c3-gateway-config.xml" ;;
        broadcast-matroska/dump_elements) echo "fixtures/mkv/vp9-opus.webm" ;;
        broadcast-matroska/list_tracks)   echo "fixtures/mkv/vp9-opus.webm" ;;
        broadcast-matroska/list_samples)  echo "fixtures/mkv/vp9-opus.webm" ;;
        broadcast-matroska/remux)         echo "fixtures/mkv/vp9-opus.webm $WORKDIR/remux.mkv" ;;
        # Both directions are keystream transforms over opaque bytes, so each
        # runs standalone against the same input — no ordering dependency
        # between the two examples (cargo enumerates targets alphabetically,
        # which would run descramble first).
        dvb-csa/scramble_file)
            echo "fixtures/ts/h264-aac.ts $WORKDIR/scrambled.ts 0102030405060708" ;;
        dvb-csa/descramble_file)
            echo "fixtures/ts/h264-aac.ts $WORKDIR/descrambled.ts 0102030405060708" ;;
        dvr-runtime/record_ts_file)
            echo "fixtures/ts/h264-aac.ts" ;;
        isobmff/parse_boxes)  echo "fixtures/mp4/progressive/av-faststart.mp4" ;;
        isobmff/parse_emsg)   echo "fixtures/scte35/scte35-emsg-v0.bin" ;;
        isobmff/list_samples) echo "fixtures/mp4/progressive/av-prog.mp4" ;;
        isobmff/list_metadata_tracks) echo "fixtures/mp4/boxes/hint-hmhd.mp4" ;;
        isobmff/print_audio_tracks) echo "fixtures/mp4/boxes/track-description.mp4" ;;
        isobmff/print_hint_tracks) echo "fixtures/mp4/boxes/hint-offsets.mp4" ;;
        isobmff-tokio/list_samples) echo "fixtures/mp4/progressive/av-prog.mp4" ;;
        isobmff-tokio/read_sample) echo "fixtures/mp4/progressive/av-prog.mp4" ;;
        isobmff/read_tags)    echo "fixtures/mp4/progressive/av-prog.mp4" ;;
        isobmff/write_tags)   echo "fixtures/mp4/progressive/av-prog.mp4 $WORKDIR/tags.udta" ;;
        transmux/transcode_pass_through) echo "$WORKDIR/transcode.cmaf" ;;
        dvr-runtime/list_programmes)
            echo "fixtures/ts/tnt-5w-12732v-isi6-10s.ts" ;;
        *) echo "" ;;
    esac
}

fail=0
checked=0
skipped=0

# Build every selected example ONCE, then run the binaries. `cargo run -p
# <crate>` per example resolved features per package, unlike the workspace
# build, and so rebuilt and relinked the dependency graph for each of the 160+
# examples. Cargo reports each executable's path; the run below uses those.
scope=(--workspace)
if [ -n "$ONLY_CRATES" ]; then
    scope=()
    for c in $ONLY_CRATES; do scope+=(-p "$c"); done
fi
EXES="$WORKDIR/executables.tsv"
if ! cargo build -q "${scope[@]}" --examples "$@" --message-format=json-render-diagnostics |
    python3 -c '
import json, os, sys
for line in sys.stdin:
    try:
        msg = json.loads(line)
    except ValueError:
        continue
    if msg.get("reason") != "compiler-artifact" or not msg.get("executable"):
        continue
    if "example" not in msg["target"]["kind"]:
        continue
    print(os.path.dirname(msg["manifest_path"]) + "\t" + msg["target"]["name"] + "\t" + msg["executable"])
' >"$EXES"; then
    echo "examples: build failed"
    exit 1
fi

while IFS=$'\t' read -r pkg ex dir; do
    [ -z "$pkg" ] && continue
    if [ -n "$ONLY_CRATES" ]; then
        case "$ONLY_CRATES" in
            *" $pkg "*) ;;
            *) continue ;;
        esac
    fi
    case "$SKIP_CRATES" in
        *" $pkg "*)
            echo "  skip  $pkg/$ex (crate excluded from this feature set)"
            skipped=$((skipped + 1))
            continue
            ;;
    esac
    case "$SERVER_EXAMPLES" in
        *" $pkg/$ex "*)
            echo "  skip  $pkg/$ex (binds a socket — build-gated only)"
            skipped=$((skipped + 1))
            continue
            ;;
    esac
    checked=$((checked + 1))
    # shellcheck disable=SC2046 # deliberate word splitting: the map holds a
    # whitespace-separated argument list, not one argument.
    exe=$(awk -F'\t' -v dir="$dir" -v ex="$ex" '$1 == dir && $2 == ex { print $3 }' "$EXES")
    if [ -z "$exe" ]; then
        echo "  FAIL  $pkg/$ex was not built"
        fail=1
        continue
    fi
    # Examples of the same name in different crates share one output path, so
    # the binary there is whichever was linked last: those run through cargo.
    if [ "$(awk -F'\t' -v exe="$exe" '$3 == exe' "$EXES" | wc -l)" -gt 1 ]; then
        out=$(cargo run -q -p "$pkg" --example "$ex" "$@" -- $(example_args "$pkg/$ex") 2>&1 | LC_ALL=C tr -d '\0')
    else
        out=$(CARGO_MANIFEST_DIR="$dir" "$exe" $(example_args "$pkg/$ex") 2>&1 | LC_ALL=C tr -d '\0')
    fi
    rc=$?

    if [ "$rc" -ne 0 ]; then
        if printf '%s' "$out" | grep -qiE 'usage:|missing (required )?arg|expects? (a|an) |provide a path'; then
            echo "  FAIL  $pkg/$ex needs arguments but declares none (issue #947)"
            echo "        Add its invocation to example_args() in $0 so it actually runs."
            printf '%s\n' "$out" | sed 's/^/        /'
            fail=1
            continue
        fi
        echo "  FAIL  $pkg/$ex (exit $rc)"
        printf '%s\n' "$out" | sed 's/^/        /'
        fail=1
        continue
    fi

    if printf '%s' "$out" | grep -qiE 'fixture not available|nothing to do'; then
        echo "  FAIL  $pkg/$ex silently skipped its own fixture (issue #947)"
        printf '%s\n' "$out" | sed 's/^/        /'
        fail=1
        continue
    fi

    echo "  ok    $pkg/$ex"
    # NOTE: no "$@" here. Target enumeration does not depend on the feature
    # set, and forwarding per-package flags like `--features whip` to a
    # workspace-wide `cargo metadata` is an error.
done < <(cargo metadata --no-deps --format-version 1 |
    python3 -c '
import json, sys
for p in json.load(sys.stdin)["packages"]:
    for t in p["targets"]:
        if "example" in t["kind"]:
            print(p["name"] + "\t" + t["name"] + "\t" + p["manifest_path"].rsplit("/", 1)[0])
')

echo "examples: $checked checked, $skipped skipped, fail=$fail"
exit $fail
