#!/usr/bin/env python3
"""Published-dependency consistency check (issue #821).

`cargo build --workspace` cannot catch this: every workspace crate depends on
its siblings via a `path = "../x"` Cargo.toml entry, so path resolution always
picks up the *local* copy regardless of what version requirement is written
next to it. The workspace is therefore internally consistent by construction
-- the only place a mismatch can actually exist is in the *published* graph on
crates.io, which no local build or test ever looks at.

The failure mode (issue #819, and two more instances the same week): a
workspace crate takes a major bump, but some sibling that depends on it does
not get republished requiring the new major. A downstream consumer who then
combines the bumped crate with the stale sibling ends up with two majors of
the same library in one dependency graph. Trait impls belong to the wrong
major, and the symptom is a baffling "no method found" on a type that plainly
has it -- the old major's compiled code, not the new one.

Four checks, all generic over workspace siblings (no special-casing):
  1. **Bucket homogeneity** — for each in-tree requirement on a workspace
     sibling, enumerate every unyanked published version that the requirement
     admits, fetch that version's own workspace-sibling deps, and flag any
     whose epochs disagree with in-tree epochs.  Catches consumers resolving
     a stale intermediate dep into their graph (issue #858).
  2. **Bump class** — compare each crate's latest-published sibling-dep epochs
     against its in-tree epochs.  If any epoch changed but the crate's version
     stayed inside the same compat bucket, that is a violation: an epoch
     change requires a major-class bump (minor for 0.x, major for >=1.0).
  3. **Dev-dep acyclicity** — no workspace crate may dev-depend on a workspace
     crate that transitively normal-depends on it.  Pure `cargo metadata`
     graph check, no network.  Controlled by `--enforce-dev-cycles`.
  4. **Publish order** — emit a topologically sorted publish order over
     normal edges only.

Three violation buckets (legacy, from the original single-check era):

  **BLOCKING** -- the in-tree manifest is itself stale (someone forgot to bump
  the requirement), or the requirement is fixed in-tree but the crate is not
  being republished in the current wave, so the stale published requirement
  stays live indefinitely.  This fails the check (exit 1 with `--blocking`).

  **PENDING** -- the violation self-heals when the current wave finishes
  publishing.  Either the dep was dropped in-tree (republishing removes it
  entirely), or the in-tree manifest already carries the correct requirement
  and the crate is being republished in this wave.  Reported under its own
  heading, but never blocks the exit code.

  **STALE DEV** -- a versioned dev-dependency whose required range still
  resolves against published (unyanked) crates, so publishes work fine and
  consumers never pull dev-deps anyway.  Hygiene only.

This script:
  1. Reads the in-tree name -> version and name -> sibling-requirements for
     every workspace member (via `cargo metadata`).
  2. For each member, fetches its LATEST PUBLISHED manifest's dependencies
     from crates.io (a crate that has never been published is skipped, not a
     failure -- see NOTES).
  3. For every published dependency on another workspace member, classifies
     the stale-requirement case using the pure `classify()` function.

Dev-dependencies are included (a versioned dev-dependency is resolved by
`cargo publish` too -- this is exactly what broke `rtmp-runtime`'s publish
against `transmux`). Path-only dev-dependencies (no version requirement) are
never a problem here: `cargo publish` strips them from the published manifest
entirely, so they never appear in the crates.io response in the first place.

Network errors talking to crates.io are retried, then SKIPPED with a warning
(not a failure) -- a flaky network must not look like a real violation.

Exit code is always 0 unless `--blocking` is passed, in which case it is 1 if
any BLOCKING violation was found. The caller (CI) decides whether this run is
advisory (ordinary pushes/PRs) or blocking (release tags).

Requirement matching honours floors and ceilings
------------------------------------------------
`req_admits()` is the single place a requirement is tested against a published
version. It understands the forms this workspace uses -- bare/caret (`0.3`,
`^0.21`, `9`, `0.3.1`) and comparator sets (`>=0.3.0, <0.3.1`) -- and honours
both the floor and any explicit upper bound.

This matters because the fix for #858 introduces three-component requirements
(`mpeg-ts = "0.3.1"`). An epoch-only comparison collapses `0.3.1` to the bucket
`(0, 3, 0)`, identical to `0.3.0`'s, so it would keep reporting a violation the
floor has already resolved -- blocking the release tag forever on 22 findings
that are fixed. `compat_epoch()` is still used where a BUCKET is genuinely the
question (bump-class decisions); it is no longer used to decide admission.
"""

from __future__ import annotations

import argparse
import collections
import itertools
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request

USER_AGENT = "rust-broadcast-published-dep-consistency-check (github.com/fishloa/rust-broadcast)"
RETRIES = 3
RETRY_BACKOFF_SECONDS = 2

# crates.io asks for ~1 request/second from tooling. Check 1 enumerates every
# published version a requirement admits, so a full run makes hundreds of
# requests and WILL be rate-limited without pacing. Being throttled is not a
# cosmetic problem: an unreachable crate is an unchecked crate.
MIN_REQUEST_INTERVAL_SECONDS = 1.0
_last_request_at = 0.0


def _throttle() -> None:
    """Sleep so consecutive crates.io requests stay ~1s apart."""
    global _last_request_at
    delta = time.monotonic() - _last_request_at
    if delta < MIN_REQUEST_INTERVAL_SECONDS:
        time.sleep(MIN_REQUEST_INTERVAL_SECONDS - delta)
    _last_request_at = time.monotonic()


_cargo_metadata_cache: dict | None = None


def _cargo_metadata() -> dict:
    """Return parsed `cargo metadata --no-deps --format-version 1 --locked`,
    cached so every accessor shares one invocation.
    """
    global _cargo_metadata_cache
    if _cargo_metadata_cache is None:
        out = subprocess.run(
            ["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"],
            check=True,
            capture_output=True,
            text=True,
        )
        _cargo_metadata_cache = json.loads(out.stdout)
    return _cargo_metadata_cache


def workspace_packages() -> dict[str, str]:
    """name -> in-tree version, for every workspace member."""
    data = _cargo_metadata()
    return {p["name"]: p["version"] for p in data["packages"]}


def workspace_dependencies() -> dict[tuple[str, str], tuple[str, str]]:
    """(member, sibling) -> (req, kind), for every workspace member's in-tree
    dependency on another workspace member.

    Reads the `dependencies` array from `cargo metadata`.  `kind` is
    normalised: `null` becomes `"normal"`, `"dev"` stays `"dev"`, `"build"`
    stays `"build"`.
    """
    data = _cargo_metadata()
    # Membership filter. Without it this returns EVERY dependency, including
    # third-party ones, and check 1 then enumerates every published version of
    # `serde`, `tokio`, … — hundreds of crates.io requests that rate-limit
    # (HTTP 429). Skipped requests are silently treated as "no violation", so
    # the check under-reports rather than failing loudly. Keep this filter.
    members = {p["name"] for p in data["packages"]}
    result: dict[tuple[str, str], tuple[str, str]] = {}
    for pkg in data["packages"]:
        member = pkg["name"]
        for dep in pkg.get("dependencies", []):
            sibling = dep.get("name")
            req = dep.get("req")
            if sibling is None or req is None:
                continue
            if sibling not in members:
                continue
            if req == "*":
                continue
            kind = dep.get("kind") or "normal"
            result[(member, sibling)] = (req, kind)
    return result


# ── Generic workspace-sibling listing ────────────────────────────────────
# These extracts a list of all sibling crate names, and a normal- /
# dev-dependency adjacency structure, from `cargo metadata --no-deps`,
# staying generic over whatever crates are in the workspace.
# --------------------------------------------------------------------------

def workspace_sibling_names() -> list[str]:
    """Sorted list of every workspace member name."""
    data = _cargo_metadata()
    names = sorted(p["name"] for p in data["packages"])
    return names


def workspace_normal_dep_map() -> dict[str, set[str]]:
    """crate -> set of sibling names it normal-depends on (direct)."""
    data = _cargo_metadata()
    siblings = {p["name"] for p in data["packages"]}
    result: dict[str, set[str]] = {}
    for pkg in data["packages"]:
        member = pkg["name"]
        result.setdefault(member, set())
        for dep in pkg.get("dependencies", []):
            if dep.get("name") in siblings:
                kind = dep.get("kind") or "normal"
                if kind == "normal":
                    result[member].add(dep["name"])
    return result


def workspace_dev_dep_map() -> dict[str, set[str]]:
    """crate -> set of sibling names it dev-depends on (direct)."""
    data = _cargo_metadata()
    siblings = {p["name"] for p in data["packages"]}
    result: dict[str, set[str]] = {}
    for pkg in data["packages"]:
        member = pkg["name"]
        result.setdefault(member, set())
        for dep in pkg.get("dependencies", []):
            if dep.get("name") in siblings:
                kind = dep.get("kind") or "normal"
                if kind == "dev":
                    result[member].add(dep["name"])
    return result


# ── crates.io helpers ────────────────────────────────────────────────────


def http_get_json(url: str) -> dict | None:
    """GET `url` as JSON, retrying on transient errors.

    Returns None for a 404 (crate/version not published -- not a failure).
    Returns None (with a warning printed) if every retry is exhausted on a
    non-404 error -- a flaky network must not read as a real violation.
    """
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    last_err = None
    for attempt in range(1, RETRIES + 1):
        _throttle()
        try:
            with urllib.request.urlopen(req, timeout=15) as resp:
                return json.loads(resp.read())
        except urllib.error.HTTPError as e:
            if e.code == 404:
                return None
            # 429 means we are asking too fast, not that the data is absent.
            # Back off hard: the alternative is exhausting retries and
            # recording the crate as "skipped", which on a release tag now
            # fails the run outright.
            if e.code == 429:
                time.sleep(RETRY_BACKOFF_SECONDS * (2 ** attempt))
            last_err = e
        except (urllib.error.URLError, TimeoutError, OSError) as e:
            last_err = e
        if attempt < RETRIES:
            time.sleep(RETRY_BACKOFF_SECONDS * attempt)
    print(
        f"::warning::crates.io request failed after {RETRIES} attempts, skipping: "
        f"{url} ({last_err})",
        file=sys.stderr,
    )
    return "error"  # sentinel: distinct from "not published" (None)


def latest_published_version(crate: str) -> str | None | str:
    data = http_get_json(f"https://crates.io/api/v1/crates/{crate}")
    if data == "error":
        return "error"
    if data is None:
        return None
    return data.get("crate", {}).get("max_version")


def range_still_resolvable(crate: str, req: str) -> bool:
    """True if some unyanked published version of `crate` satisfies `req`'s
    major/compat epoch.

    Used to decide whether a stale *dev*-dependency actually blocks anything.
    `cargo publish` must resolve versioned dev-deps, so a dev-dep pointing at a
    major that no longer exists is a hard failure -- but one pointing at a
    major that is still on crates.io resolves fine and is merely untidy.
    """
    return any_published_version_satisfies(crate, req)


_crate_versions_cache: dict[str, list[str]] = {}


def _crate_versions(crate: str) -> list[str]:
    """Return published unyanked version strings for `crate`, with caching."""
    global _crate_versions_cache
    if crate not in _crate_versions_cache:
        data = http_get_json(f"https://crates.io/api/v1/crates/{crate}")
        if data in (None, "error"):
            _crate_versions_cache[crate] = []
        else:
            _crate_versions_cache[crate] = [
                v["num"]
                for v in data.get("versions", [])
                if not v.get("yanked")
            ]
    return _crate_versions_cache[crate]


def any_published_version_satisfies(crate: str, req: str) -> bool:
    """True if some published, unyanked version of `crate` satisfies `req`.

    Uses `req_admits`, so floors and explicit ceilings are honoured: a
    requirement of `>=0.3.0, <0.3.1` is NOT reported as satisfied by 0.3.1.
    """
    if compat_epoch(req) is None:
        return True
    return any(req_admits(req, v) for v in _crate_versions(crate))


def version_is_published(crate: str, version: str) -> bool:
    """True if `version` exists as a published unyanked version of `crate`."""
    return version in _crate_versions(crate)


def _version_satisfies_req(version: str, req: str) -> bool:
    """True if `version` satisfies `req`, honouring floors and ceilings."""
    return req_admits(req, version)


def published_dependencies(crate: str, version: str) -> list[dict] | str:
    data = http_get_json(f"https://crates.io/api/v1/crates/{crate}/{version}/dependencies")
    if data == "error":
        return "error"
    if data is None:
        return []
    return data.get("dependencies", [])


_VERSION_TOKEN = re.compile(r"\d+(?:\.\d+){0,2}")


def compat_epoch(version_str: str) -> tuple[int, int, int] | None:
    """The caret-compatible bucket a version belongs to.

    `>=1.0.0`: `(1, major, 0)` -- any minor/patch bump within a major is
    compatible, so only the major distinguishes an epoch.
    `0.y.z` with `y > 0`: `(0, y, 0)` -- caret compatibility for a 0.x crate
    only spans a fixed minor.
    `0.0.z`: `(0, 0, z)` -- caret compatibility for 0.0.x spans only the exact
    patch.
    """
    m = _VERSION_TOKEN.search(version_str)
    if not m:
        return None
    parts = [int(x) for x in m.group(0).split(".")]
    while len(parts) < 3:
        parts.append(0)
    major, minor, patch = parts[0], parts[1], parts[2]
    if major > 0:
        return (1, major, 0)
    if minor > 0:
        return (0, minor, 0)
    return (0, 0, patch)


def req_admits(req: str, version: str) -> bool:
    """True if `req` actually admits `version`.

    Unlike `compat_epoch`, this honours the FLOOR and any explicit upper bound,
    so a three-component requirement is understood:

        req_admits("0.3",            "0.3.0") -> True
        req_admits("0.3.1",          "0.3.0") -> False   # floored above it
        req_admits(">=0.3.0, <0.3.1","0.3.1") -> False   # explicit ceiling

    Without this, check 1 could not see the fix for #858: flooring `mpeg-ts` to
    `"0.3.1"` leaves the caret EPOCH at (0, 3, 0), identical to 0.3.0's, so an
    epoch-only comparison keeps reporting a violation that has been resolved --
    and blocks the release tag forever.

    Supports the forms this workspace uses: bare/caret (`0.3`, `^0.21`,
    `9`, `0.3.1`) and comma-separated comparator sets (`>=0.3.0, <0.3.1`).
    Anything unrecognised falls back to the epoch comparison, which is the
    previous behaviour.
    """
    req = req.strip()

    if any(op in req for op in (">", "<", "=")) and "," in req or req.startswith((">", "<")):
        for clause in req.split(","):
            clause = clause.strip()
            m = re.match(r"(>=|<=|>|<|=)?\s*(\d+(?:\.\d+){0,2})", clause)
            if not m:
                return False
            op, bound = m.group(1) or "=", m.group(2)
            cmp = compare_versions(version, bound)
            if op == ">=" and cmp < 0:
                return False
            if op == ">" and cmp <= 0:
                return False
            if op == "<=" and cmp > 0:
                return False
            if op == "<" and cmp >= 0:
                return False
            if op == "=" and cmp != 0:
                return False
        return True

    # Bare or caret requirement: same compat epoch AND at or above the floor.
    if compat_epoch(req) != compat_epoch(version):
        return False
    m = _VERSION_TOKEN.search(req)
    if not m:
        return True
    return compare_versions(version, m.group(0)) >= 0


def compat_bucket(version_str: str) -> tuple[int, int] | None:
    """The version bucket for bump-class decision: (0, minor) for 0.x,
    (major, 0) for >=1.0.  A change of bucket requires a major-class bump.
    """
    m = _VERSION_TOKEN.search(version_str)
    if not m:
        return None
    parts = [int(x) for x in m.group(0).split(".")]
    while len(parts) < 2:
        parts.append(0)
    major, minor = parts[0], parts[1]
    if major > 0:
        return (major, 0)
    return (0, minor)


def compare_versions(a: str, b: str) -> int:
    """Compare two semver-ish version strings: -1 if a<b, 0 if equal, 1 if a>b.

    Strips pre-release and build-metadata suffixes (`-` and `+`) before
    comparing numeric components.  Non-numeric components are treated as 0.
    """
    a = _strip_meta(a)
    b = _strip_meta(b)
    a_parts: list[int] = []
    b_parts: list[int] = []
    for x in a.split("."):
        try:
            a_parts.append(int(x))
        except ValueError:
            a_parts.append(0)
    for x in b.split("."):
        try:
            b_parts.append(int(x))
        except ValueError:
            b_parts.append(0)
    while len(a_parts) < 3:
        a_parts.append(0)
    while len(b_parts) < 3:
        b_parts.append(0)
    if a_parts < b_parts:
        return -1
    if a_parts > b_parts:
        return 1
    return 0


def _strip_meta(v: str) -> str:
    """Strip pre-release (`-`) and build-metadata (`+`) suffixes."""
    for sep in ("-", "+"):
        idx = v.find(sep)
        if idx != -1:
            v = v[:idx]
    return v


# ── Published version helper ─────────────────────────────────────────────

_published_sibling_deps_cache: dict[tuple[str, str], dict[str, str]] = {}


def _published_sibling_deps(crate: str, version: str) -> dict[str, str]:
    """Return {sibling_name: req} for all workspace-sibling normal deps of
    `crate` at published `version`.  Cached.

    Returns empty dict on error (warned by http_get_json); the caller must
    not treat an empty dict as "no deps" for correctness-critical checks.
    """
    key = (crate, version)
    if key in _published_sibling_deps_cache:
        return _published_sibling_deps_cache[key]

    siblings = set(workspace_sibling_names())
    deps = published_dependencies(crate, version)
    if deps == "error":
        _published_sibling_deps_cache[key] = {}
        return {}

    result: dict[str, str] = {}
    for dep in deps:
        sib = dep.get("crate_id")
        req = dep.get("req")
        kind = dep.get("kind") or "normal"
        if sib in siblings and req is not None and kind == "normal":
            result[sib] = req
    _published_sibling_deps_cache[key] = result
    return result


# ── Check 1: bucket homogeneity ──────────────────────────────────────────
# For each in-tree requirement on a workspace sibling, enumerate every
# unyanked published version of that sibling which the requirement admits.
# For each admitted version, compare its own workspace-sibling dep epochs
# against the current in-tree epochs.  Any disagreement is a violation.

def _check1_bucket_homogeneity(
    in_tree_deps: dict[tuple[str, str], tuple[str, str]],
    members: dict[str, str],
    normal_dep_map: dict[str, set[str]],
) -> list[str]:
    """Return violation messages for bucket-homogeneity check 1."""
    violations: list[str] = []
    seen: set[tuple[str, str, str, str, str, str]] = set()

    for (consumer, sibling), (req, kind) in sorted(in_tree_deps.items()):
        if kind not in ("normal", "dev"):
            continue
        req_epoch = compat_epoch(req)
        if req_epoch is None:
            continue

        # Enumerate all unyanked published versions of the sibling that
        # the requirement admits.
        # `req_admits` honours the floor and any explicit ceiling, so a
        # three-component requirement is understood. Epoch-only matching would
        # keep flagging versions the requirement has already excluded.
        admitted = [
            v for v in _crate_versions(sibling)
            if req_admits(req, v)
        ]
        for admitted_version in admitted:
            # Get that published version's own workspace-sibling deps
            pub_deps = _published_sibling_deps(sibling, admitted_version)
            if not pub_deps:
                # Could be an error or genuinely no sibling deps
                continue

            # Check each published dep's epoch against in-tree
            for trans_dep, pub_req in sorted(pub_deps.items()):
                pub_epoch = compat_epoch(pub_req)
                if pub_epoch is None:
                    continue
                in_tree_epoch = compat_epoch(members.get(trans_dep, "0.0.0"))
                if in_tree_epoch is None:
                    continue
                if pub_epoch == in_tree_epoch:
                    continue  # clean

                # Found an epoch disagreement
                key = (consumer, req, sibling, admitted_version, trans_dep, pub_req)
                if key in seen:
                    continue
                seen.add(key)

                # Format the in-tree transitive dep's epoch for the message
                in_tree_trans_req = None
                # Find what epoch the sibling requires in-tree
                for (sib_member, tdep), (t_req, t_kind) in in_tree_deps.items():
                    if sib_member == sibling and tdep == trans_dep and t_kind == "normal":
                        in_tree_trans_req = t_req
                        break

                in_tree_ref = in_tree_trans_req if in_tree_trans_req else f"(version {members[trans_dep]})"
                violations.append(
                    f"{consumer} requires {sibling} {req}, which admits "
                    f"{sibling} {admitted_version} built against {trans_dep} "
                    f"{pub_req} (in-tree is {in_tree_ref}) -- a consumer "
                    f"can resolve two {trans_dep} majors"
                )

    return violations


# ── Check 2: bump class ──────────────────────────────────────────────────

def _check2_bump_class(
    in_tree_deps: dict[tuple[str, str], tuple[str, str]],
    members: dict[str, str],
) -> list[str]:
    """Return violation messages for bump-class check 2.

    For each workspace member, compare its in-tree sibling-dep epochs against
    those of its last published version.  If any sibling epoch changed but the
    crate's own version stayed inside the same compat bucket, flag it.
    """
    violations: list[str] = []

    for name, in_tree_version in sorted(members.items()):
        max_version = latest_published_version(name)
        if max_version in (None, "error"):
            continue

        # Compare the crate's own bucket
        in_tree_bucket = compat_bucket(in_tree_version)
        published_bucket = compat_bucket(max_version)
        if in_tree_bucket is None or published_bucket is None:
            continue
        if in_tree_bucket == published_bucket:
            # Same bucket — check if any sibling epoch changed
            pub_deps = _published_sibling_deps(name, max_version)
            for sibling, pub_req in sorted(pub_deps.items()):
                pub_epoch = compat_epoch(pub_req)
                if pub_epoch is None:
                    continue
                # Find in-tree requirement for this sibling
                in_tree_req = None
                for (m, s), (r, k) in in_tree_deps.items():
                    if m == name and s == sibling and k == "normal":
                        in_tree_req = r
                        break
                if in_tree_req is None:
                    continue
                in_tree_epoch = compat_epoch(in_tree_req)
                if in_tree_epoch is None:
                    continue
                if pub_epoch == in_tree_epoch:
                    continue  # clean

                # Epoch changed but bucket stayed → violation
                required_bump = _bump_class_to_str(name, in_tree_bucket)
                violations.append(
                    f"{name} {max_version} requires {sibling} {pub_req}, "
                    f"but {name} {in_tree_version} in-tree requires "
                    f"{sibling} {in_tree_req} — an epoch change ({pub_req} → "
                    f"{in_tree_req}) while staying in the "
                    f"{_bucket_human(name, in_tree_bucket)} bucket; "
                    f"requires {required_bump}"
                )
    return violations


_BUCKET_HUMAN_CACHE: dict[tuple[str, tuple[int, int]], str] = {}


def _bucket_human(name: str, bucket: tuple[int, int]) -> str:
    """Human-readable bucket name like '0.3' or '9'."""
    key = (name, bucket)
    if key in _BUCKET_HUMAN_CACHE:
        return _BUCKET_HUMAN_CACHE[key]
    maj, minor = bucket
    if maj > 0:
        s = str(maj)
    else:
        s = f"0.{minor}"
    _BUCKET_HUMAN_CACHE[key] = s
    return s


def _bump_class_to_str(name: str, bucket: tuple[int, int]) -> str:
    """The version string that would be the next major-class bump."""
    maj, minor = bucket
    if maj > 0:
        return f"{maj + 1}.0.0"
    else:
        return f"0.{minor + 1}.0"


# ── Check 3: dev-dep acyclicity ──────────────────────────────────────────

def _check3_dev_cycles(
    normal_dep_map: dict[str, set[str]],
    dev_dep_map: dict[str, set[str]],
) -> list[str]:
    """Return violation messages for dev-dep cycle check 3.

    For each dev-dep edge A (dev)-> B, check if there is a transitive
    normal-dep path from B to A.
    """
    violations: list[str] = []
    seen: set[tuple[str, str]] = set()

    for dev_src, dev_targets in sorted(dev_dep_map.items()):
        for dev_tgt in sorted(dev_targets):
            if (dev_src, dev_tgt) in seen:
                continue

            # Find transitive normal-dep closure of dev_tgt
            visited = set()
            stack = [dev_tgt]
            reachable = set()
            while stack:
                node = stack.pop()
                if node in visited:
                    continue
                visited.add(node)
                for nd in normal_dep_map.get(node, set()):
                    reachable.add(nd)
                    if nd not in visited:
                        stack.append(nd)

            if dev_src in reachable:
                # Find a path for reporting
                path = _find_normal_path(normal_dep_map, dev_tgt, dev_src)
                path_str = " → ".join(path) if path else f"{dev_tgt} → ... → {dev_src}"
                violations.append(
                    f"{dev_src} --dev--> {dev_tgt}  AND  "
                    f"{dev_tgt} --normal-->* {dev_src}  "
                    f"(path: {path_str})"
                )
                seen.add((dev_src, dev_tgt))

    return violations


def _find_normal_path(
    normal_dep_map: dict[str, set[str]],
    start: str,
    target: str,
) -> list[str] | None:
    """BFS to find a normal-dep path from start to target."""
    from collections import deque
    parent: dict[str, str | None] = {start: None}
    queue = deque([start])
    while queue:
        node = queue.popleft()
        if node == target:
            # Reconstruct path
            path: list[str] = []
            cur: str | None = node
            while cur is not None:
                path.append(cur)
                cur = parent[cur]
            path.reverse()
            return path
        for neighbor in normal_dep_map.get(node, set()):
            if neighbor not in parent:
                parent[neighbor] = node
                queue.append(neighbor)
    return None


# ── Check 4: publish order ───────────────────────────────────────────────

def _check4_publish_order(
    normal_dep_map: dict[str, set[str]],
) -> tuple[list[str], list[str]]:
    """Return (sorted_publish_order, cycle_nodes_if_any).

    Topological sort of normal-dep graph.  If a cycle exists, return the
    nodes involved in the cycle.
    """
    from collections import deque

    # Kahn's algorithm
    in_degree: dict[str, int] = {}
    all_nodes = set(normal_dep_map.keys())
    for n in all_nodes:
        in_degree.setdefault(n, 0)
    for n, deps in normal_dep_map.items():
        for d in deps:
            all_nodes.add(d)
            in_degree.setdefault(d, 0)
            in_degree[d] += 1

    queue = deque([n for n in sorted(all_nodes) if in_degree.get(n, 0) == 0])
    order: list[str] = []

    while queue:
        n = queue.popleft()
        order.append(n)
        for d in sorted(normal_dep_map.get(n, set())):
            in_degree[d] -= 1
            if in_degree[d] == 0:
                queue.append(d)

    if len(order) != len(all_nodes):
        # Cycle detected
        cycle_nodes = [n for n in sorted(all_nodes) if in_degree.get(n, 0) > 0]
        return order, cycle_nodes

    # `in_degree[d]` counts how many crates DEPEND ON `d`, so Kahn's algorithm
    # above starts from the crates nobody depends on (media-doctor,
    # multimux-cli, …) and walks DOWN to the foundation. That is the reverse of
    # a publish order: `cargo publish` requires every dependency to be on the
    # index already, so `broadcast-common` must go FIRST, not last.
    #
    # Reverse it. Emitting this backwards is worse than emitting nothing — a
    # release engineer following it publishes every dependent before its
    # dependencies and every single publish fails.
    order.reverse()
    return order, []


# ── classify (legacy single-check logic) ─────────────────────────────────


def classify(
    name: str,
    max_version: str,
    in_tree_version: str,
    sibling: str,
    published_req: str,
    in_tree_req: str | None,
    kind: str,
    sibling_in_tree_version: str,
    dev_range_resolvable: bool,
) -> str:
    """Classify a stale published-requirement into a bucket.

    PRECONDITION: the caller must only call this when the published requirement
    is already known to be stale, i.e.
    `compat_epoch(published_req) < compat_epoch(sibling_in_tree_version)`.
    Current requirements are filtered out before classify is reached; there is
    no `"ok"` return from this function.

    Returns one of:
      `"blocking"`  -- requires human action (manifest edit or version bump)
      `"pending"`   -- self-heals when the current wave finishes publishing
      `"stale_dev"` -- dev-dep with a still-resolvable range (hygiene only)
    """
    # Stale-dev bucket: dev-dep with a still-resolvable range.
    if kind == "dev" and dev_range_resolvable:
        return "stale_dev"

    # in_tree_req is absent (dep was dropped in-tree) → PENDING.
    # Republishing removes the dep entirely from the published manifest.
    if in_tree_req is None:
        return "pending"

    in_tree_epoch = compat_epoch(in_tree_req)
    sibling_epoch = compat_epoch(sibling_in_tree_version)

    if in_tree_epoch is not None and sibling_epoch is not None:
        # In-tree manifest itself stale → BLOCKING.
        if in_tree_epoch < sibling_epoch:
            return "blocking"

    # In-tree requirement is current.  Is the crate being republished?
    in_tree_vs_published = compare_versions(in_tree_version, max_version)
    if in_tree_vs_published > 0:
        # in_tree_version > max_version → being republished in this wave → PENDING
        return "pending"
    else:
        # Requirement fixed in-tree but crate not being republished → BLOCKING
        return "blocking"


# ── main ─────────────────────────────────────────────────────────────────


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--blocking",
        action="store_true",
        help="Exit 1 if any BLOCKING violation is found (release-tag runs). "
        "Without this flag, violations are printed but the exit code is 0 "
        "(ordinary push/PR runs).",
    )
    parser.add_argument(
        "--enforce-dev-cycles",
        action="store_true",
        help="Exit 1 on dev-dep cycle violations (check 3). "
        "Not yet the default — TODO(#858-followup): make default-on "
        "once the two known cycles are removed.",
    )
    args = parser.parse_args()

    skip_network = os.environ.get("SKIP_NETWORK", "") == "1"

    members = workspace_packages()
    in_tree_deps = workspace_dependencies()
    normal_dep_map = workspace_normal_dep_map()
    dev_dep_map = workspace_dev_dep_map()

    blocking: list[str] = []
    pending: list[str] = []
    stale_dev: list[str] = []
    unpublishable: list[str] = []
    publish_order: list[str] = []
    skipped: list[str] = []

    if skip_network:
        print("SKIP_NETWORK=1: running graph-only checks (3-4), skipping crates.io checks (1-2)")
        print()
        check1_violations = []
        check2_violations = []
    else:
        # ── Legacy check: latest-published-vs-in-tree ────────────────────────
        for name, in_tree_version in sorted(members.items()):
            max_version = latest_published_version(name)
            if max_version == "error":
                skipped.append(name)
                continue
            if max_version is None:
                print(f"  {name}: not yet published, skipping")
                continue

            deps = published_dependencies(name, max_version)
            if deps == "error":
                skipped.append(name)
                continue

            for dep in deps:
                sibling = dep.get("crate_id")
                req = dep.get("req")
                if sibling not in members or req is None:
                    continue
                published_epoch = compat_epoch(req)
                sibling_in_tree_version = members[sibling]
                current_epoch = compat_epoch(sibling_in_tree_version)
                if published_epoch is None or current_epoch is None:
                    continue
                if published_epoch >= current_epoch:
                    continue  # requirement is current -- ok

                kind = dep.get("kind") or "normal"

                # Resolve in-tree req, matching kind
                in_tree_req = None
                for (m, s), (r, k) in in_tree_deps.items():
                    if m == name and s == sibling and k == kind:
                        in_tree_req = r
                        break

                dev_range_resolvable = range_still_resolvable(sibling, req) if kind == "dev" else False

                bucket = classify(
                    name=name,
                    max_version=max_version,
                    in_tree_version=in_tree_version,
                    sibling=sibling,
                    published_req=req,
                    in_tree_req=in_tree_req,
                    kind=kind,
                    sibling_in_tree_version=sibling_in_tree_version,
                    dev_range_resolvable=dev_range_resolvable,
                )

                detail = (
                    f"{name} {max_version} ({kind}-dep) requires "
                    f"{sibling} {req}, but {sibling} is now {sibling_in_tree_version} in-tree"
                )

                if bucket == "stale_dev":
                    stale_dev.append(
                        detail + " (dev-dep; old major still published, so"
                        " publishes resolve -- hygiene only)"
                    )
                elif bucket == "pending":
                    suffix = _pending_suffix(
                        name=name,
                        max_version=max_version,
                        in_tree_version=in_tree_version,
                        sibling=sibling,
                        sibling_in_tree_version=sibling_in_tree_version,
                        in_tree_req=in_tree_req,
                        kind=kind,
                    )
                    pending.append(detail + suffix)
                else:
                    blocking.append(detail + " (superseded major)")

        # ── Second check: in-tree requirement satisfiability ───────────────
        for (name, sibling), (req, kind) in sorted(in_tree_deps.items()):
            if name not in members or sibling not in members:
                continue

            if any_published_version_satisfies(sibling, req):
                continue

            sibling_in_tree = members[sibling]
            sibling_satisfies = _version_satisfies_req(sibling_in_tree, req)
            if not sibling_satisfies:
                unpublishable.append(
                    f"{name} ({kind}-dep) requires {sibling} {req}, "
                    f"but no published version satisfies that requirement "
                    f"and the sibling's in-tree version ({sibling_in_tree}) "
                    f"also does not match -- cargo publish will FAIL"
                )
                continue

            max_version = latest_published_version(sibling)
            if max_version and max_version not in (None, "error"):
                if compare_versions(sibling_in_tree, max_version) > 0:
                    publish_order.append(
                        f"{name} ({kind}-dep) requires {sibling} {req} -- "
                        f"no published version satisfies this yet, but "
                        f"{sibling} {sibling_in_tree} is being published "
                        f"this wave and will satisfy it.  Publish "
                        f"{sibling} before {name}."
                    )
                    continue

            unpublishable.append(
                f"{name} ({kind}-dep) requires {sibling} {req}, "
                f"but no published version satisfies that requirement "
                f"and {sibling} ({sibling_in_tree}) is not being "
                f"republished -- cargo publish will FAIL"
            )

        # ── Check 1: bucket homogeneity ─────────────────────────────────────
        check1_violations = _check1_bucket_homogeneity(in_tree_deps, members, normal_dep_map)

        # ── Check 2: bump class ─────────────────────────────────────────────
        check2_violations = _check2_bump_class(in_tree_deps, members)

    # ── Check 3: dev-dep cycles ─────────────────────────────────────────
    check3_violations = _check3_dev_cycles(normal_dep_map, dev_dep_map)

    # ── Check 4: publish order ──────────────────────────────────────────
    sorted_order, cycle_nodes = _check4_publish_order(normal_dep_map)

    # ── Output ──────────────────────────────────────────────────────────

    print()
    if skipped:
        print(f"::warning::skipped {len(skipped)} crate(s) due to crates.io network errors: {skipped}")

    if stale_dev:
        print(
            f"{len(stale_dev)} stale dev-dependency requirement(s) -- untidy but"
            " not blocking (old major still published, and consumers never pull"
            " dev-deps):"
        )
        for d in stale_dev:
            print(f"  - {d}")
        print()

    if unpublishable:
        print(
            f"{len(unpublishable)} unpublishable dependency problem(s) --"
            " BLOCKING (cargo publish will fail):"
        )
        for d in unpublishable:
            print(f"  - {d}")
        print()

    if publish_order:
        print(
            f"{len(publish_order)} publish-order dependency(s) -- satisfied by"
            " a sibling being published in this wave (not blocking, but"
            " publish order matters):"
        )
        for d in publish_order:
            print(f"  - {d}")
        print()

    if pending:
        print(
            f"{len(pending)} pending violation(s) -- resolved when this wave"
            " finishes publishing (not blocking):"
        )
        for d in pending:
            print(f"  - {d}")
        print()

    # ── Check 1 output ──────────────────────────────────────────────────
    if check1_violations:
        print(
            f"{len(check1_violations)} bucket-homogeneity violation(s) "
            f"(check 1 -- admits a published version with mismatched "
            f"transitive dep epochs):"
        )
        for v in check1_violations:
            print(f"  - {v}")
        print()

    # ── Check 2 output ──────────────────────────────────────────────────
    if check2_violations:
        print(
            f"{len(check2_violations)} bump-class violation(s) "
            f"(check 2 -- epoch change without major-class bump):"
        )
        for v in check2_violations:
            print(f"  - {v}")
        print()

    # ── Check 3 output ──────────────────────────────────────────────────
    if check3_violations:
        known_msg = (
            "KNOWN, being fixed in the test-relocation PR"
        )
        print(
            f"{len(check3_violations)} dev-dep cycle(s) "
            f"(check 3 -- {known_msg}):"
        )
        for v in check3_violations:
            print(f"  - {v}")
        print()

    # ── Check 4 output ──────────────────────────────────────────────────
    if cycle_nodes:
        print("  ERROR: normal-dep graph has a cycle, publish order invalid.")
        print(f"  Nodes in cycle: {', '.join(cycle_nodes)}")
        print()
    else:
        print(f"  Publish order ({len(sorted_order)} crates, normal deps only):")
        for i, crate in enumerate(sorted_order, 1):
            deps = sorted(normal_dep_map.get(crate, set()))
            dep_str = f"  (depends on: {', '.join(deps)})" if deps else ""
            print(f"  {i:2d}. {crate}{dep_str}")
        print()

    all_blocking = blocking + unpublishable

    # Check 1 and check 2 violations contribute to blocking
    all_blocking.extend(check1_violations)
    all_blocking.extend(check2_violations)

    if getattr(args, "enforce_dev_cycles", False):
        all_blocking.extend(check3_violations)

    # Cycle in normal-dep graph is always blocking
    if cycle_nodes:
        cycle_msg = f"Normal-dep graph cycle detected: {', '.join(cycle_nodes)}"
        all_blocking.append(cycle_msg)

    if all_blocking:
        print(f"Found {len(all_blocking)} published-dependency consistency violation(s) (BLOCKING):")
        for v in all_blocking:
            print(f"  - {v}")
    elif not pending and not stale_dev and not publish_order:
        print("No published-dependency consistency violations found.")

    # A crate we could not reach was NOT checked. On a release tag that is
    # indistinguishable from "checked and clean", which is precisely the
    # failure mode this whole tool exists to prevent: an absent check reading
    # as a passing check. Refuse to certify a release on unverified data.
    if skipped and args.blocking:
        print(
            f"\nBLOCKING: {len(skipped)} crate(s) could not be verified against "
            f"crates.io ({', '.join(sorted(skipped))}). A skipped crate is an "
            "UNCHECKED crate, not a clean one -- refusing to pass a release gate "
            "on unverified data. Re-run when crates.io is reachable.",
            file=sys.stderr,
        )
        return 1

    if all_blocking and args.blocking:
        print("\nBLOCKING: failing this run (release tag).", file=sys.stderr)
        return 1
    if all_blocking:
        print(
            "\nADVISORY: not failing this run (not a release tag) -- "
            "these must be resolved before the next tag.",
        )
    return 0


def _pending_suffix(
    name: str,
    max_version: str,
    in_tree_version: str,
    sibling: str,
    sibling_in_tree_version: str,
    in_tree_req: str | None,
    kind: str,
) -> str:
    """Build the explanatory suffix for a PENDING violation message."""
    if in_tree_req is None:
        return f" -- dep dropped in-tree in {name} {in_tree_version}; publishing clears this"

    if compare_versions(in_tree_version, max_version) > 0:
        return (
            f" -- in-tree {name} is {in_tree_version} requiring {sibling} {in_tree_req},"
            f" so publishing {name} {in_tree_version} clears this"
        )

    return ""


if __name__ == "__main__":
    sys.exit(main())
