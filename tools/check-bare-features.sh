#!/usr/bin/env bash
# Per-crate bare-feature compile gate: every workspace member's tests must
# compile with `--no-default-features`, one invocation per crate so workspace
# feature unification cannot re-enable what a crate alone would not have
# (rtmp-runtime's secret_hygiene.rs failed this and the unified lane hid it).
# A crate with a `std` feature is checked with `--features std`. A crate whose
# lib.rs declares `no_std` is ALSO checked with no features at all, so its test
# modules must compile against `alloc` alone (or be gated on `std`).
#
# Every optional feature of the crates in `each_feature_alone` is then built by
# itself: `--all-features` unifies them, so an import or an item left behind by
# ONE feature being off is invisible to every other lane (multimux built
# `--features whip` with an unused `RouteConfig` import, #4083). A feature that
# genuinely needs another declares it in [features] rather than being skipped
# here (multimux's `test-hooks` implies `whip`, whose seams it re-exports).
#
# Usage: tools/check-bare-features.sh [crate...]   (default: every member)
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

each_feature_alone=(multimux)

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

# Every non-`default` feature of one crate, built one invocation at a time. The
# feature list comes from the manifest via cargo metadata so a new feature is
# covered without editing this script.
check_each_feature_alone() {
    local c=$1 f
    local feats
    mapfile -t feats < <(cargo metadata --no-deps --format-version 1 --locked | python3 -c '
import json, sys
for p in json.load(sys.stdin)["packages"]:
    if p["name"] == sys.argv[1]:
        print("\n".join(sorted(f for f in p["features"] if f != "default")))
' "$c")
    for f in ${feats[@]+"${feats[@]}"}; do
        echo "::group::$c (--no-default-features --features $f)"
        cargo check -p "$c" --all-targets --no-default-features --features "$f" --locked \
            || { echo "::error::$c fails with only '$f'"; fail=1; }
        echo "::endgroup::"
    done
}

for c in "${each_feature_alone[@]}"; do
    if [ "$#" -gt 0 ]; then
        case " $* " in *" $c "*) ;; *) continue ;; esac
    fi
    check_each_feature_alone "$c"
done
exit "$fail"
