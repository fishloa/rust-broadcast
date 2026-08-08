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
# Examples that REQUIRE a CLI argument are skipped (they exit non-zero by design
# when invoked bare, and print usage). They remain build-gated.
#
# Usage: tools/run-examples.sh [extra cargo args...]
set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

# Examples that bind a socket or otherwise wait on the network. They are
# long-running servers by design, not self-terminating demos, so running them
# here would hang or collide on a port. Still build-gated.
SERVER_EXAMPLES=" rtmp-runtime/capture_publish multimux/serve_rtsp "

# Crates whose examples cannot be built under this invocation's feature set /
# toolchain. `webrtc-runtime` is here because `--all-features` enables its
# `media` feature, whose dependency graph requires rustc >= 1.88 while the
# workspace MSRV is 1.86 — `--all-features` has no per-crate opt-out, so the
# crate is excluded here and covered by its own CI job instead.
SKIP_CRATES=" webrtc-runtime "

fail=0
checked=0
skipped=0

while IFS=$'\t' read -r pkg ex; do
    [ -z "$pkg" ] && continue
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
    out=$(cargo run -q -p "$pkg" --example "$ex" "$@" 2>&1)
    rc=$?

    if [ "$rc" -ne 0 ]; then
        if printf '%s' "$out" | grep -qiE 'usage:|missing (required )?arg|expects? (a|an) |provide a path'; then
            echo "  skip  $pkg/$ex (needs an argument)"
            skipped=$((skipped + 1))
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
done < <(cargo metadata --no-deps --format-version 1 "$@" |
    python3 -c '
import json, sys
for p in json.load(sys.stdin)["packages"]:
    for t in p["targets"]:
        if "example" in t["kind"]:
            print(p["name"] + "\t" + t["name"])
')

echo "examples: $checked checked, $skipped skipped (need args), fail=$fail"
exit $fail
