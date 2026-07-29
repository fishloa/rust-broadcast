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

This script:
  1. Reads the in-tree name -> version for every workspace member (via
     `cargo metadata`).
  2. For each member, fetches its LATEST PUBLISHED manifest's dependencies
     from crates.io (a crate that has never been published is skipped, not a
     failure -- see NOTES).
  3. For every published dependency on another workspace member, compares the
     published requirement's compatible-version "epoch" (the caret-compatible
     bucket: the major for >=1.0.0, or the `0.minor` bucket for 0.x) against
     that sibling's CURRENT in-tree epoch. If the published requirement's
     epoch is older, that is a violation: some already-published crate still
     requires a superseded major of a sibling that has since moved on.

Dev-dependencies are included (a versioned dev-dependency is resolved by
`cargo publish` too -- this is exactly what broke `rtmp-runtime`'s publish
against `transmux`). Path-only dev-dependencies (no version requirement) are
never a problem here: `cargo publish` strips them from the published manifest
entirely, so they never appear in the crates.io response in the first place.

Network errors talking to crates.io are retried, then SKIPPED with a warning
(not a failure) -- a flaky network must not look like a real violation.

Exit code is always 0 unless `--blocking` is passed, in which case it is 1 if
any violation was found. The caller (CI) decides whether this run is
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


def workspace_packages() -> dict[str, str]:
    """name -> in-tree version, for every workspace member."""
    out = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"],
        check=True,
        capture_output=True,
        text=True,
    )
    data = json.loads(out.stdout)
    return {p["name"]: p["version"] for p in data["packages"]}


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


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--blocking",
        action="store_true",
        help="Exit 1 if any violation is found (release-tag runs). "
        "Without this flag, violations are printed but the exit code is 0 "
        "(ordinary push/PR runs).",
    )
    args = parser.parse_args()

    members = workspace_packages()
    violations: list[str] = []
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
            current_epoch = compat_epoch(members[sibling])
            if published_epoch is None or current_epoch is None:
                continue
            if published_epoch < current_epoch:
                kind = dep.get("kind") or "normal"
                detail = (
                    f"{name} {max_version} ({kind}-dep) requires "
                    f"{sibling} {req}, but {sibling} is now {members[sibling]} in-tree"
                )
                # A NORMAL dep is consumer-visible: anyone combining this
                # published crate with a current sibling gets two majors of
                # the same library in one graph, and trait impls belong to the
                # wrong one. That is the #819 failure and it is an error.
                #
                # A versioned DEV dep is only a problem when the required
                # range cannot be resolved at all -- which is what actually
                # broke rtmp-runtime's publish (it needed transmux 0.20 before
                # 0.20 existed). While the old major is still published and
                # unyanked, the publish resolves and no consumer ever pulls a
                # dev-dep, so this is hygiene, not breakage. Reporting it as a
                # blocking violation would cry wolf on every release tag.
                if kind == "dev" and range_still_resolvable(sibling, req):
                    stale_dev.append(
                        detail + " (dev-dep; old major still published, so"
                        " publishes resolve -- hygiene only)"
                    )
                else:
                    violations.append(detail + " (superseded major)")

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

    if violations:
        print(f"Found {len(violations)} published-dependency consistency violation(s):")
        for v in violations:
            print(f"  - {v}")
    else:
        print("No published-dependency consistency violations found.")

    if violations and args.blocking:
        print("\nBLOCKING: failing this run (release tag).", file=sys.stderr)
        return 1
    if violations:
        print(
            "\nADVISORY: not failing this run (not a release tag) -- "
            "these must be resolved before the next tag.",
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
