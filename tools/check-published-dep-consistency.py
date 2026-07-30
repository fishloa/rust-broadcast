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

Three violation buckets:

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
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request

USER_AGENT = "rust-broadcast-published-dep-consistency-check (github.com/fishloa/rust-broadcast)"
RETRIES = 3
RETRY_BACKOFF_SECONDS = 2


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
    result: dict[tuple[str, str], tuple[str, str]] = {}
    for pkg in data["packages"]:
        member = pkg["name"]
        for dep in pkg.get("dependencies", []):
            sibling = dep.get("name")
            req = dep.get("req")
            if sibling is None or req is None:
                continue
            if req == "*":
                continue
            kind = dep.get("kind") or "normal"
            result[(member, sibling)] = (req, kind)
    return result


def http_get_json(url: str) -> dict | None:
    """GET `url` as JSON, retrying on transient errors.

    Returns None for a 404 (crate/version not published -- not a failure).
    Returns None (with a warning printed) if every retry is exhausted on a
    non-404 error -- a flaky network must not read as a real violation.
    """
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    last_err = None
    for attempt in range(1, RETRIES + 1):
        try:
            with urllib.request.urlopen(req, timeout=15) as resp:
                return json.loads(resp.read())
        except urllib.error.HTTPError as e:
            if e.code == 404:
                return None
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
    epoch = compat_epoch(req)
    if epoch is None:
        return True
    data = http_get_json(f"https://crates.io/api/v1/crates/{crate}")
    if data in (None, "error"):
        # Unknown: assume resolvable so a network blip cannot manufacture a
        # blocking violation.
        return True
    for v in data.get("versions", []):
        if v.get("yanked"):
            continue
        if compat_epoch(v.get("num", "")) == epoch:
            return True
    return False


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

    Returns one of:
      `"blocking"`  -- requires human action (manifest edit or version bump)
      `"pending"`   -- self-heals when the current wave finishes publishing
      `"stale_dev"` -- dev-dep with a still-resolvable range (hygiene only)
      `"ok"`        -- no violation (published requirement is current)

    The caller determines the published requirement is stale *before* calling:
    `compat_epoch(published_req) < compat_epoch(sibling_in_tree_version)` must
    already hold.
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


def compare_versions(a: str, b: str) -> int:
    """Compare two semver-ish version strings: -1 if a<b, 0 if equal, 1 if a>b."""
    a_parts = [int(x) for x in a.split(".")]
    b_parts = [int(x) for x in b.split(".")]
    while len(a_parts) < 3:
        a_parts.append(0)
    while len(b_parts) < 3:
        b_parts.append(0)
    if a_parts < b_parts:
        return -1
    if a_parts > b_parts:
        return 1
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--blocking",
        action="store_true",
        help="Exit 1 if any BLOCKING violation is found (release-tag runs). "
        "Without this flag, violations are printed but the exit code is 0 "
        "(ordinary push/PR runs).",
    )
    args = parser.parse_args()

    members = workspace_packages()
    in_tree_deps = workspace_dependencies()

    blocking: list[str] = []
    pending: list[str] = []
    stale_dev: list[str] = []
    skipped: list[str] = []

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

    if pending:
        print(
            f"{len(pending)} pending violation(s) -- resolved when this wave"
            " finishes publishing (not blocking):"
        )
        for d in pending:
            print(f"  - {d}")
        print()

    if blocking:
        print(f"Found {len(blocking)} published-dependency consistency violation(s) (BLOCKING):")
        for v in blocking:
            print(f"  - {v}")
    elif not pending:
        print("No published-dependency consistency violations found.")

    if blocking and args.blocking:
        print("\nBLOCKING: failing this run (release tag).", file=sys.stderr)
        return 1
    if blocking:
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
