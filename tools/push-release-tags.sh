#!/usr/bin/env bash
# Push release tags ONE AT A TIME, verifying each reaches crates.io before the
# next.
#
# Why this exists (issue #933): five release tags were once pushed in a single
# burst. GitHub triggered **no** workflow runs for any of them, nothing was
# published, and the failure was silent — `git push --tags` reported success,
# the tags existed, and five crates were believed shipped for weeks while
# every downstream `^` dependency quietly resolved to the older line.
#
# GitHub does not guarantee a workflow run per tag when many arrive at once.
# The only reliable protection is: push one, confirm the crate actually
# appears in the sparse index, then push the next.
#
# Usage:
#   tools/push-release-tags.sh <tag> [<tag> ...]
#   tools/push-release-tags.sh --dry-run <tag> [...]
#
# Each tag must be of the form <crate>-v<version> (or v<version> for the
# lockstep tag, which this script rejects — publish that one deliberately).
#
# NOTE: this script pushes tags, which triggers publication. It does not
# publish directly; CI is still the only thing that runs `cargo publish`.
set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

DRY=0
if [ "${1:-}" = "--dry-run" ]; then DRY=1; shift; fi
[ $# -eq 0 ] && { echo "usage: $0 [--dry-run] <tag>..."; exit 2; }

UA='rust-broadcast-release (see Cargo.toml repository)'
# Wait up to this long for a published version to appear in the sparse index.
TIMEOUT_SECS=900
POLL_SECS=20

# crates.io sparse-index path: 1/2/3-char names are special-cased.
index_path() {
    local n="$1" len=${#1}
    if   [ "$len" -le 2 ]; then echo "$len/$n"
    elif [ "$len" -eq 3 ]; then echo "3/${n:0:1}/$n"
    else echo "${n:0:2}/${n:2:2}/$n"; fi
}

live_version() {
    curl -s -H "User-Agent: $UA" "https://index.crates.io/$(index_path "$1")" |
        python3 -c '
import json, sys
vs = [json.loads(l)["vers"] for l in sys.stdin if l.strip()]
print(vs[-1] if vs else "UNPUBLISHED")' 2>/dev/null || echo ERR
}

fail=0
for tag in "$@"; do
    case "$tag" in
        v*[!-]*) if [[ "$tag" =~ ^v[0-9] ]]; then
            echo "REFUSING $tag — that is the five-crate lockstep tag."
            echo "  Push it deliberately; this script is for single-crate tags."
            fail=1; continue
        fi ;;
    esac

    crate="${tag%-v*}"
    want="${tag##*-v}"
    if [ "$crate" = "$tag" ] || [ -z "$want" ]; then
        echo "SKIP $tag — not of the form <crate>-v<version>"; fail=1; continue
    fi

    manifest="$crate/Cargo.toml"
    [ -f "$manifest" ] || { echo "SKIP $tag — no $manifest"; fail=1; continue; }
    have=$(grep -m1 '^version' "$manifest" | sed 's/.*"\(.*\)".*/\1/')
    if [ "$have" != "$want" ]; then
        echo "REFUSING $tag — $manifest says $have, tag says $want"
        fail=1; continue
    fi

    before=$(live_version "$crate")
    if [ "$before" = "$want" ]; then
        echo "SKIP $tag — $crate $want is already on crates.io"; continue
    fi

    echo "=== $tag  ($crate: $before -> $want) ==="
    if [ "$DRY" = "1" ]; then echo "  [dry-run] would: git push origin $tag"; continue; fi

    git push origin "$tag" || { echo "  push failed"; fail=1; continue; }

    echo -n "  waiting for the sparse index"
    waited=0
    while [ "$waited" -lt "$TIMEOUT_SECS" ]; do
        sleep "$POLL_SECS"; waited=$((waited + POLL_SECS))
        now=$(live_version "$crate")
        if [ "$now" = "$want" ]; then echo " -> published ($want)"; break; fi
        echo -n "."
    done

    if [ "$(live_version "$crate")" != "$want" ]; then
        echo ""
        # Distinguish the two very different reasons for not appearing:
        #   (a) a workflow is sitting on the crates-io environment gate,
        #       waiting for a human reviewer — expected, not a failure;
        #   (b) no workflow ran at all — the #933 failure mode.
        # Reporting (a) as (b) trains the operator to ignore the warning,
        # which is how #933 went unnoticed in the first place.
        state=$(gh run list --limit 1 --workflow="Release $crate" \
                    --json status --jq '.[0].status' 2>/dev/null || echo unknown)
        if [ "$state" = "waiting" ]; then
            echo "  $crate $want is NOT published yet — its release workflow is"
            echo "  WAITING on the crates-io environment approval gate."
            echo "  That is expected: a human reviewer must approve the publish."
            echo "    gh run list --limit 1 --workflow=\"Release $crate\" --json url"
            echo "  Approve it, confirm the crate appears in the index, then re-run"
            echo "  this script for the remaining tags."
            exit 2
        fi
        echo "  TIMED OUT after ${TIMEOUT_SECS}s — $crate is still $(live_version "$crate"),"
        echo "  and no workflow is awaiting approval (last run status: $state)."
        echo "  THIS IS THE #933 FAILURE MODE — the tag was pushed and nothing ran."
        echo "    gh run list --limit 10"
        echo "  Do NOT push the remaining tags until this one is resolved."
        exit 1
    fi
done

exit $fail
