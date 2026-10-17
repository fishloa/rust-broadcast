#!/usr/bin/env bash
# Push release tags ONE AT A TIME; after each, wait until the crate is on the
# crates.io index AND its release workflow has finished green, before the next.
#
# Why this exists (issue #933): five release tags were once pushed in a single
# burst. GitHub triggered **no** workflow runs for any of them, nothing was
# published, and the failure was silent -- `git push --tags` reported success,
# the tags existed, and five crates were believed shipped for weeks while
# every downstream `^` dependency quietly resolved to the older line.
#
# Usage:
#   tools/push-release-tags.sh [--dry-run] <tag> [<tag> ...]
#   tools/push-release-tags.sh [--dry-run] --plan <pairs-file>
#
# --plan FILE   take the tags from `tools/prepare-release.py --tag-order` over
#               the pair list (topological: broadcast-common first; versioned
#               dev-dependency edges count; the lockstep five are ONE `v<ver>`
#               slot). The lockstep tag is never pushed by this script: when
#               the plan reaches it and it is not live yet, the script stops
#               (exit 2) so you push it by hand, approve it, and re-run.
#               Re-running is safe: crates already live are skipped.
# --dry-run     print the exact order and the preflight verdicts, touching
#               nothing: no git command, no network, no push. A manifest that is
#               not bumped yet is a warning in a dry run (it is a hard error in
#               a real run).
#
# Tags are <crate>-v<version>. An explicit `v<version>` (the lockstep tag) is
# REFUSED: push that one deliberately, by hand.
#
# Exit status (truthful; the old script could exit 0 with tags never pushed
# because it pressed on after a refused/failed tag and trusted a single index
# poll):
#   0  every tag is pushed (or already live) AND its release workflow succeeded
#   1  anything failed: preflight refusal, push failure, index never showed the
#      version, a release workflow failed/was cancelled, index/gh unreadable.
#      Nothing after the failing tag is pushed.
#   2  stopped, not failed: a release run is WAITING on the `crates-io`
#      environment approval gate, or the plan reached the lockstep tag. Approve
#      / push by hand, then re-run the same command.
#   64 usage error
#
# Environment (tests and tuning): REMOTE (origin), INDEX_BASE
# (https://index.crates.io), POLL_SECS (20), MAX_POLLS (45 = 15 min), RUN_POLLS
# (45), PREPARE_RELEASE (tools/prepare-release.py).
#
# NOTE: this script pushes tags, which triggers publication. It does not
# publish directly; CI is still the only thing that runs `cargo publish`.
set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

DRY=0
PLAN=""
TAGS=()
while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run) DRY=1 ;;
        --plan) shift; PLAN="${1:-}"; [ -n "$PLAN" ] || { echo "--plan needs a file"; exit 64; } ;;
        -*) echo "unknown option $1"; exit 64 ;;
        *) TAGS+=("$1") ;;
    esac
    shift
done

REMOTE="${REMOTE:-origin}"
INDEX_BASE="${INDEX_BASE:-https://index.crates.io}"
POLL_SECS="${POLL_SECS:-20}"
MAX_POLLS="${MAX_POLLS:-45}"
RUN_POLLS="${RUN_POLLS:-45}"
PREPARE_RELEASE="${PREPARE_RELEASE:-tools/prepare-release.py}"
UA='rust-broadcast-release (see Cargo.toml repository)'

if [ -n "$PLAN" ]; then
    [ ${#TAGS[@]} -eq 0 ] || { echo "give --plan OR explicit tags, not both"; exit 64; }
    [ -f "$PLAN" ] || { echo "no such pair file: $PLAN"; exit 64; }
    # shellcheck disable=SC2046
    order=$(python3 "$PREPARE_RELEASE" --tag-order $(cat "$PLAN")) || { echo "could not compute the tag order"; exit 1; }
    while IFS= read -r t; do [ -n "$t" ] && TAGS+=("$t"); done <<<"$order"
fi
[ ${#TAGS[@]} -gt 0 ] || { echo "usage: $0 [--dry-run] (--plan <pairs-file> | <tag>...)"; exit 64; }

# crates.io sparse-index path: 1/2/3-char names are special-cased.
index_path() {
    local n="$1" len=${#1}
    if   [ "$len" -le 2 ]; then echo "$len/$n"
    elif [ "$len" -eq 3 ]; then echo "3/${n:0:1}/$n"
    else echo "${n:0:2}/${n:2:2}/$n"; fi
}

# Prints the newest published version, UNPUBLISHED (a real 404), or ERR (any
# other failure: never mistaken for "not published").
live_version() {
    local body code
    body=$(curl -s -w '\n%{http_code}' -H "User-Agent: $UA" "$INDEX_BASE/$(index_path "$1")") || { echo ERR; return; }
    code="${body##*$'\n'}"
    body="${body%$'\n'*}"
    case "$code" in
        404) echo UNPUBLISHED; return ;;
        200) ;;
        *) echo ERR; return ;;
    esac
    printf '%s\n' "$body" | python3 -c '
import json, sys
vs = [json.loads(l)["vers"] for l in sys.stdin if l.strip()]
print(vs[-1] if vs else "UNPUBLISHED")' 2>/dev/null || echo ERR
}

manifest_version() { grep -m1 '^version' "$1/Cargo.toml" 2>/dev/null | sed 's/.*"\(.*\)".*/\1/'; }

is_lockstep_tag() { [[ "$1" =~ ^v[0-9] ]]; }

# Newest version of the lockstep tag's LAST-published crate (dvb-tools).
lockstep_live() { [ "$(live_version dvb-tools)" = "${1#v}" ]; }

# ---- preflight: validate EVERY tag before anything is pushed ------------------
errors=0
warnings=0
echo "Plan (${#TAGS[@]} tag(s), one at a time, in this order):"
n=0
for tag in "${TAGS[@]}"; do
    n=$((n + 1))
    if is_lockstep_tag "$tag"; then
        if [ -z "$PLAN" ]; then
            echo "  $n. $tag  REFUSED: the lockstep tag is pushed by hand, never by this script"
            errors=$((errors + 1))
        else
            echo "  $n. $tag  MANUAL: push by hand, approve, re-run"
        fi
        continue
    fi
    crate="${tag%-v*}"
    want="${tag##*-v}"
    if [ "$crate" = "$tag" ] || [ -z "$want" ]; then
        echo "  $n. $tag  REFUSED: not of the form <crate>-v<version>"
        errors=$((errors + 1)); continue
    fi
    have=$(manifest_version "$crate")
    if [ -z "$have" ]; then
        echo "  $n. $tag  REFUSED: no $crate/Cargo.toml"
        errors=$((errors + 1)); continue
    fi
    if [ "$have" != "$want" ]; then
        if [ "$DRY" = "1" ]; then
            echo "  $n. $tag  warning: $crate/Cargo.toml says $have (not bumped in this tree)"
            warnings=$((warnings + 1))
        else
            echo "  $n. $tag  REFUSED: $crate/Cargo.toml says $have, tag says $want"
            errors=$((errors + 1))
        fi
        continue
    fi
    if [ "$DRY" = "0" ] && ! git rev-parse -q --verify "refs/tags/$tag^{commit}" >/dev/null; then
        echo "  $n. $tag  REFUSED: no LOCAL tag $tag (create all tags locally at the release commit first)"
        errors=$((errors + 1)); continue
    fi
    echo "  $n. $tag"
done

if [ "$errors" -gt 0 ]; then
    echo "PREFLIGHT FAILED ($errors problem(s)); nothing was pushed."
    exit 1
fi
if [ "$DRY" = "1" ]; then
    echo "[dry-run] $warnings warning(s); no git command, no network, nothing pushed."
    exit 0
fi
command -v gh >/dev/null || { echo "gh not found: cannot confirm the release workflows; refusing to push."; exit 1; }

# ---- the loop ------------------------------------------------------------------
# State of this tag's "Release*" workflow run(s): "<status> <conclusion>" lines.
run_states() {
    gh run list --branch "$1" --limit 20 --json workflowName,status,conclusion \
        --jq '.[] | select(.workflowName | startswith("Release")) | "\(.status) \(.conclusion)"' 2>/dev/null
}

summary=()
for tag in "${TAGS[@]}"; do
    if is_lockstep_tag "$tag"; then
        if lockstep_live "$tag"; then
            echo "=== $tag: lockstep already live (dvb-tools ${tag#v})"; summary+=("$tag live"); continue
        fi
        echo "=== $tag: the lockstep tag is NOT live. Push it BY HAND, approve its crates-io gate,"
        echo "    wait until dvb-tools ${tag#v} is on the index, then re-run this command."
        echo "    (every tag after it needs it live first)"
        exit 2
    fi

    crate="${tag%-v*}"
    want="${tag##*-v}"
    before=$(live_version "$crate")
    if [ "$before" = "ERR" ]; then
        echo "FAIL $tag: could not read the crates.io index for $crate (not treating that as 'unpublished')."
        exit 1
    fi
    if [ "$before" = "$want" ]; then
        echo "SKIP $tag: $crate $want is already on crates.io"; summary+=("$tag already-live"); continue
    fi

    echo "=== $tag  ($crate: $before -> $want) ==="
    if ! git push "$REMOTE" "refs/tags/$tag"; then
        echo "FAIL $tag: git push failed. Later tags were NOT pushed."
        exit 1
    fi

    echo -n "  waiting for the sparse index"
    polls=0
    published=0
    while [ "$polls" -lt "$MAX_POLLS" ]; do
        sleep "$POLL_SECS"; polls=$((polls + 1))
        states=$(run_states "$tag")
        if grep -qE '^completed (failure|cancelled|timed_out|startup_failure|action_required)' <<<"$states"; then
            echo ""
            echo "FAIL $tag: its release workflow FAILED ($(tr '\n' ';' <<<"$states"))."
            echo "    gh run list --branch $tag"
            exit 1
        fi
        if grep -q '^waiting' <<<"$states"; then
            echo ""
            echo "STOP $tag: its release run is WAITING on the crates-io environment approval gate."
            echo "    gh run list --branch $tag --json databaseId,status,url"
            echo "    Approve it (docs/RELEASE-RUNBOOK-2026-10.md), then re-run this same command."
            exit 2
        fi
        if [ "$(live_version "$crate")" = "$want" ]; then published=1; echo " -> on the index ($want)"; break; fi
        echo -n "."
    done
    if [ "$published" != "1" ]; then
        echo ""
        echo "FAIL $tag: TIMED OUT; $crate is still $(live_version "$crate") and no run awaits approval."
        echo "    THIS IS THE #933 FAILURE MODE (tag pushed, nothing ran): gh run list --branch $tag"
        echo "    Do NOT push the remaining tags until this one is resolved."
        exit 1
    fi

    echo -n "  waiting for the release workflow to finish"
    polls=0
    done_ok=0
    while [ "$polls" -lt "$RUN_POLLS" ]; do
        states=$(run_states "$tag")
        if grep -qE '^completed success' <<<"$states" && ! grep -qvE '^completed success' <<<"$states"; then
            done_ok=1; echo " -> success"; break
        fi
        if grep -qE '^completed (failure|cancelled|timed_out|startup_failure|action_required)' <<<"$states"; then
            echo ""
            echo "FAIL $tag: published, but its release workflow FAILED ($(tr '\n' ';' <<<"$states"))."
            exit 1
        fi
        sleep "$POLL_SECS"; polls=$((polls + 1)); echo -n "."
    done
    if [ "$done_ok" != "1" ]; then
        echo ""
        echo "FAIL $tag: published, but no successful completed release workflow was seen (last: $(tr '\n' ';' <<<"$states"))."
        exit 1
    fi
    summary+=("$tag published")
done

echo ""
echo "All ${#TAGS[@]} tag(s) done:"
printf '  %s\n' "${summary[@]}"
exit 0
