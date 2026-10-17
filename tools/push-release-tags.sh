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
#   tools/push-release-tags.sh [--allow-tag-commit <sha>]... ...
#
# --plan FILE   take the tags from `tools/prepare-release.py --tag-order` over
#               the pair list (topological: broadcast-common first; versioned
#               dev-dependency edges count; the lockstep five are ONE `v<ver>`
#               slot). The lockstep tag is never pushed by this script: when
#               the plan reaches it and it is not live yet, the script stops
#               (exit 2) so you push it by hand, approve it, and re-run.
#               Re-running is safe: crates already live are skipped.
# --allow-tag-commit SHA
#               a local tag must point at HEAD; name a pinned release commit
#               here (repeatable) to allow a tag that points elsewhere.
# --dry-run     print the exact order and the preflight verdicts, touching
#               nothing: no git command, no network, no push. A manifest that is
#               not bumped yet is a warning in a dry run (it is a hard error in
#               a real run).
#
# Preflight (real run, before ANY push) reads the TAGGED tree, not the working
# tree: `git show <tag>:<crate>/Cargo.toml` must carry the tag's version and
# `git show <tag>:docs/release-notes/<crate>-<version>.md` must exist, be
# non-empty and not `status: DRAFT`. The local tag must point at HEAD (or an
# --allow-tag-commit SHA). A tag that already exists on the remote at a
# DIFFERENT commit is refused (a stale remote tag makes the push a silent no-op,
# issue #933); one at the same commit is only allowed to RESUME when a Release
# workflow run exists for it, and is refused otherwise.
#
# Before each push (plan AND explicit-tag mode; in plan mode also the lockstep
# slot, over all five lockstep crates' external deps) every workspace
# dependency in the TAGGED manifest (normal, build, target tables, versioned
# dev-deps) must be live on the crates.io index at a version satisfying the
# manifest's REQUIREMENT (max semver, yanked ignored). Otherwise exit 2 naming
# the missing dependency; nothing is pushed.
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
#      environment approval gate, the plan reached the lockstep tag, or a
#      workspace dependency is not live at the required version yet. Approve /
#      push by hand / wait, then re-run the same command.
#   64 usage error
#
# Environment (tests and tuning): REMOTE (origin), INDEX_BASE
# (https://index.crates.io), POLL_SECS (20), MAX_POLLS (45 = 15 min), RUN_POLLS
# (45), PREPARE_RELEASE (tools/prepare-release.py), RELEASE_DEPS
# (tools/release_deps.py).
#
# NOTE: this script pushes tags, which triggers publication. It does not
# publish directly; CI is still the only thing that runs `cargo publish`.
set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

DRY=0
PLAN=""
TAGS=()
ALLOW_COMMITS=()
while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run) DRY=1 ;;
        --plan) shift; PLAN="${1:-}"; [ -n "$PLAN" ] || { echo "--plan needs a file"; exit 64; } ;;
        --allow-tag-commit) shift; [ -n "${1:-}" ] || { echo "--allow-tag-commit needs a sha"; exit 64; }; ALLOW_COMMITS+=("$1") ;;
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
RELEASE_DEPS="${RELEASE_DEPS:-tools/release_deps.py}"
UA='rust-broadcast-release (see Cargo.toml repository)'
LOCKSTEP_CRATES=(dvb-bbframe dvb-si dvb-conformance dvb-t2mi dvb-tools)
LOCKSTEP_CSV="dvb-bbframe,dvb-si,dvb-conformance,dvb-t2mi,dvb-tools"

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

# Fetch the sparse-index body: sets IDX_CODE and IDX_BODY (code 000 on a curl failure).
IDX_CODE=""
IDX_BODY=""
index_fetch() {
    local body
    body=$(curl -s -w '\n%{http_code}' -H "User-Agent: $UA" "$INDEX_BASE/$(index_path "$1")") || { IDX_CODE=000; IDX_BODY=""; return; }
    IDX_CODE="${body##*$'\n'}"
    IDX_BODY="${body%$'\n'*}"
}

# Prints the newest (MAX SEMVER, yanked ignored) published version, UNPUBLISHED
# (a real 404), or ERR (any other failure: never mistaken for "not published").
# The index lists versions in publish order, so the LAST line is not the newest
# after a patch release of an older line.
live_version() {
    index_fetch "$1"
    case "$IDX_CODE" in
        404) echo UNPUBLISHED; return ;;
        200) ;;
        *) echo ERR; return ;;
    esac
    printf '%s\n' "$IDX_BODY" | python3 "$RELEASE_DEPS" max 2>/dev/null || echo ERR
}

# 0 = a live, unyanked version satisfies <req>; 1 = none does; 3 = unreadable.
# Prints the reason on a non-zero return.
dep_live() {
    local crate="$1" req="$2" out
    index_fetch "$crate"
    case "$IDX_CODE" in
        404) echo "$crate $req: crate is not on crates.io at all"; return 1 ;;
        200) ;;
        *) echo "$crate $req: could not read the crates.io index (HTTP $IDX_CODE)"; return 3 ;;
    esac
    if out=$(printf '%s\n' "$IDX_BODY" | python3 "$RELEASE_DEPS" satisfies "$req" 2>&1); then
        return 0
    fi
    echo "$crate $req: no live version satisfies it (live: ${out#NO })"
    return 1
}

# Every workspace dependency of the given crates, as declared in <ref>'s tree,
# must be live at the declared requirement. <skip> = comma list of crates that
# publish in the same lane. Prints the first missing one. 0 ok, 1 missing, 3 unreadable.
deps_live_at() {
    local ref="$1" skip="$2"
    shift 2
    local c manifest dep req line rc
    for c in "$@"; do
        manifest=$(git show "$ref:$c/Cargo.toml" 2>/dev/null) || { echo "$c: no $c/Cargo.toml in $ref"; return 1; }
        while read -r dep req; do
            [ -n "$dep" ] || continue
            line=$(dep_live "$dep" "$req"); rc=$?
            if [ "$rc" -ne 0 ]; then echo "$c depends on $line"; return "$rc"; fi
        done < <(printf '%s\n' "$manifest" | python3 "$RELEASE_DEPS" deps --skip "$skip")
    done
    return 0
}

manifest_version() { grep -m1 '^version' "$1/Cargo.toml" 2>/dev/null | sed 's/.*"\(.*\)".*/\1/'; }

is_lockstep_tag() { [[ "$1" =~ ^v[0-9] ]]; }

# Newest version of the lockstep tag's LAST-published crate (dvb-tools).
lockstep_live() { [ "$(live_version dvb-tools)" = "${1#v}" ]; }

REMOTE_TAGS=""
# The commit a tag points at on the remote ("" when the remote has no such tag).
remote_sha() {
    printf '%s\n' "$REMOTE_TAGS" | awk -v r="refs/tags/$1" '$2 == r "^{}" { p = $1 } $2 == r { u = $1 } END { print (p != "" ? p : u) }'
}

commit_allowed() {
    local c="$1" a full
    [ "$c" = "$HEAD_SHA" ] && return 0
    for a in "${ALLOW_COMMITS[@]+"${ALLOW_COMMITS[@]}"}"; do
        full=$(git rev-parse -q --verify "$a^{commit}" 2>/dev/null) || continue
        [ "$c" = "$full" ] && return 0
    done
    return 1
}

# ---- preflight: validate EVERY tag before anything is pushed ------------------
errors=0
warnings=0
HEAD_SHA=""
if [ "$DRY" = "0" ]; then
    HEAD_SHA=$(git rev-parse -q --verify 'HEAD^{commit}') || { echo "not in a git repository with a HEAD commit"; exit 1; }
    REMOTE_TAGS=$(git ls-remote --tags "$REMOTE") || { echo "PREFLIGHT FAILED: could not read the tags of remote '$REMOTE' (git ls-remote); nothing was pushed."; exit 1; }
fi
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
    if [ "$DRY" = "1" ]; then
        have=$(manifest_version "$crate")
        if [ -z "$have" ]; then
            echo "  $n. $tag  REFUSED: no $crate/Cargo.toml"
            errors=$((errors + 1)); continue
        fi
        if [ "$have" != "$want" ]; then
            echo "  $n. $tag  warning: $crate/Cargo.toml says $have (not bumped in this tree)"
            warnings=$((warnings + 1)); continue
        fi
        echo "  $n. $tag"
        continue
    fi

    # real run: everything is read from the TAG, never from the working tree
    tagc=$(git rev-parse -q --verify "refs/tags/$tag^{commit}" 2>/dev/null)
    if [ -z "$tagc" ]; then
        echo "  $n. $tag  REFUSED: no LOCAL tag $tag (create all tags locally at the release commit first)"
        errors=$((errors + 1)); continue
    fi
    if ! commit_allowed "$tagc"; then
        echo "  $n. $tag  REFUSED: tag points at ${tagc:0:12} but HEAD is ${HEAD_SHA:0:12} (check out the release commit, or pass --allow-tag-commit <sha>)"
        errors=$((errors + 1)); continue
    fi
    have=$(git show "$tag:$crate/Cargo.toml" 2>/dev/null | python3 "$RELEASE_DEPS" version 2>/dev/null)
    if [ -z "$have" ]; then
        echo "  $n. $tag  REFUSED: no readable $crate/Cargo.toml in the tagged tree"
        errors=$((errors + 1)); continue
    fi
    if [ "$have" != "$want" ]; then
        echo "  $n. $tag  REFUSED: $crate/Cargo.toml at the tag says $have, tag says $want"
        errors=$((errors + 1)); continue
    fi
    note="docs/release-notes/$crate-$want.md"
    if ! why=$(git show "$tag:$note" 2>/dev/null | python3 "$RELEASE_DEPS" note-ok); then
        echo "  $n. $tag  REFUSED: release note $note at the tag is missing/empty/DRAFT (${why:-missing})"
        errors=$((errors + 1)); continue
    fi
    rsha=$(remote_sha "$tag")
    if [ -n "$rsha" ] && [ "$rsha" != "$tagc" ]; then
        echo "  $n. $tag  REFUSED: remote $REMOTE already has $tag at ${rsha:0:12} (local ${tagc:0:12}); a stale remote tag makes the push a silent no-op (#933)"
        errors=$((errors + 1)); continue
    fi
    echo "  $n. $tag"
done

if [ "$errors" -gt 0 ]; then
    echo "PREFLIGHT FAILED ($errors problem(s)); nothing was pushed."
    exit 1
fi
if [ "$DRY" = "1" ]; then
    echo "[dry-run] $warnings warning(s); no git command, no network, nothing pushed (dependency liveness is checked in a real run)."
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
        ref="refs/tags/$tag"
        git rev-parse -q --verify "$ref^{commit}" >/dev/null 2>&1 || ref=HEAD
        if ! missing=$(deps_live_at "$ref" "$LOCKSTEP_CSV" "${LOCKSTEP_CRATES[@]}"); then
            echo "=== $tag: STOP, a dependency of the lockstep five is not live at the required version:"
            echo "    $missing"
            echo "    Do NOT push $tag yet. Wait for it (or fix the plan), then re-run this command."
            exit 2
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

    missing=$(deps_live_at "refs/tags/$tag" "" "$crate"); rc=$?
    if [ "$rc" -eq 1 ]; then
        echo "STOP $tag: a workspace dependency is not live on crates.io at the required version:"
        echo "    $missing"
        echo "    Nothing was pushed for $tag. Publish that dependency first, then re-run this command."
        exit 2
    elif [ "$rc" -ne 0 ]; then
        echo "FAIL $tag: could not verify the dependencies: $missing"
        exit 1
    fi

    echo "=== $tag  ($crate: $before -> $want) ==="
    if [ -n "$(remote_sha "$tag")" ]; then
        # The tag is already on the remote and the crate is not live: only a RESUME
        # (a Release run exists for it) is legitimate. No run = #933, a silent no-op.
        if [ -z "$(run_states "$tag")" ]; then
            echo "FAIL $tag: the tag is already on $REMOTE but NO release workflow run exists for it (#933); pushing again is a no-op."
            echo "    gh run list --branch $tag"
            exit 1
        fi
        echo "  tag already on $REMOTE with a release run: resuming without a push"
    elif ! git push "$REMOTE" "refs/tags/$tag"; then
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
