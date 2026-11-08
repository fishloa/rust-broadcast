#!/usr/bin/env bash
# Per-crate bare-feature compile gate: every workspace member's tests must
# compile with `--no-default-features`, one invocation per crate so workspace
# feature unification cannot re-enable what a crate alone would not have
# (rtmp-runtime's secret_hygiene.rs failed this and the unified lane hid it).
# A crate with a `std` feature is checked with `--features std`. A crate whose
# lib.rs declares `no_std` is ALSO checked with no features at all, so its test
# modules must compile against `alloc` alone (or be gated on `std`).
#
# Usage: tools/check-bare-features.sh [crate...]   (default: every member)
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

members=$(cargo metadata --no-deps --format-version 1 --locked | python3 -c '
import json, os, sys
meta = json.load(sys.stdin)
ids = set(meta["workspace_members"])
for p in sorted(meta["packages"], key=lambda p: p["name"]):
    if p["id"] in ids:
        lib = os.path.join(os.path.dirname(p["manifest_path"]), "src", "lib.rs")
        try:
            nostd = "no_std" in open(lib).read()
        except OSError:
            nostd = False
        print(p["name"], "std" if "std" in p["features"] else "-", "nostd" if nostd else "-")
')

fail=0
while read -r c has_std is_nostd; do
    if [ "$#" -gt 0 ]; then
        case " $* " in *" $c "*) ;; *) continue ;; esac
    fi
    if [ "$has_std" = "std" ]; then
        feats=(--features std)
    else
        feats=()
    fi
    echo "::group::$c (--no-default-features ${feats[*]:-})"
    cargo check -p "$c" --tests --no-default-features ${feats[@]+"${feats[@]}"} --locked || { echo "::error::$c fails bare"; fail=1; }
    echo "::endgroup::"
    if [ "$is_nostd" = "nostd" ] && [ "$has_std" = "std" ]; then
        echo "::group::$c (--no-default-features, no std)"
        cargo check -p "$c" --tests --no-default-features --locked || { echo "::error::$c fails bare without std"; fail=1; }
        echo "::endgroup::"
    fi
done <<< "$members"
exit "$fail"
