#!/usr/bin/env bash
# Per-crate bare-feature compile gate: every workspace member's tests must
# compile with `--no-default-features`, one invocation per crate so workspace
# feature unification cannot re-enable what a crate alone would not have
# (rtmp-runtime's secret_hygiene.rs failed this and the unified lane hid it).
# A crate with a `std` feature is checked with `--features std`: a no_std
# crate's test harness cannot compile without std, and its real bare-metal
# build is the ci.yml `no-std` job's.
#
# Usage: tools/check-bare-features.sh [crate...]   (default: every member)
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

members=$(cargo metadata --no-deps --format-version 1 --locked | python3 -c '
import json, sys
meta = json.load(sys.stdin)
ids = set(meta["workspace_members"])
for p in sorted(meta["packages"], key=lambda p: p["name"]):
    if p["id"] in ids:
        print(p["name"], "std" if "std" in p["features"] else "-")
')

fail=0
while read -r c has_std; do
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
done <<< "$members"
exit "$fail"
