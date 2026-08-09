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
        atsc3/parse_lls)      echo "fixtures/atsc3/slt-lls-2019-01-07.bin" ;;
        atsc3/parse_slt)      echo "fixtures/atsc3/slt-lls-2019-01-07.bin" ;;
        st2022/parse_hbrmt)   echo "fixtures/st2022/st2022-6-hbrmt-payload-header.bin" ;;
        dvb-mabr/parse_session) echo "fixtures/dvb-mabr/annex-c3-gateway-config.xml" ;;
        # Both directions are keystream transforms over opaque bytes, so each
        # runs standalone against the same input — no ordering dependency
        # between the two examples (cargo enumerates targets alphabetically,
        # which would run descramble first).
        dvb-csa/scramble_file)
            echo "fixtures/atsc3/slt-lls-2019-01-07.bin $WORKDIR/scrambled.bin 0102030405060708" ;;
        dvb-csa/descramble_file)
            echo "fixtures/atsc3/slt-lls-2019-01-07.bin $WORKDIR/descrambled.bin 0102030405060708" ;;
        *) echo "" ;;
    esac
}

fail=0
checked=0
skipped=0

while IFS=$'\t' read -r pkg ex; do
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
    out=$(cargo run -q -p "$pkg" --example "$ex" "$@" -- $(example_args "$pkg/$ex") 2>&1)
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
            print(p["name"] + "\t" + t["name"])
')

echo "examples: $checked checked, $skipped skipped, fail=$fail"
exit $fail
