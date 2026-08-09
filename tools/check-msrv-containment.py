#!/usr/bin/env python3
"""Guard the MSRV containment split in .github/workflows/ci.yml.

Some crates opt into dependency graphs with a rustc floor ABOVE the workspace
MSRV, behind optional features:

    webrtc-runtime/media  -> rcgen 0.14.8, time 0.3.55 (rustc 1.88)
    multimux/whip         -> webrtc-runtime/media (same floor)

`--all-features` has no per-crate opt-out, so ANY `--all-features` invocation
whose package scope reaches one of those crates fails on the MSRV toolchain
with "rustc 1.86.0 is not supported by the following packages".

That exact failure has now been introduced four separate times, each time in a
different CI step, each time only discovered by a red run:

  - the six workspace MSRV lanes
  - the `no_std` cross-build lane
  - `tools/run-examples.sh --all-features`
  - the golden-gate job

Each fix was correct and none of them stopped the next one. This script is the
check that does: it reads the workflow and fails if a new `--all-features`
invocation reaches a floor crate without pinning a new-enough toolchain.

Usage: tools/check-msrv-containment.py [path-to-ci.yml]
Exit 0 = contained, 1 = a leak (message names the job, step and command).
"""

from __future__ import annotations

import re
import sys

# Crates whose OPTIONAL features exceed the workspace MSRV. Keep in step with
# each crate's Cargo.toml; adding a crate here is how a new 1.88-floor feature
# gets covered.
FLOOR_CRATES = {"webrtc-runtime", "multimux"}

# A command is exempt when it explicitly selects a toolchain newer than the
# MSRV: `cargo +1.97.1 …`, `cargo +$CANARY_RUST …`, `cargo +${{ env.X }} …`.
PINNED_TOOLCHAIN = re.compile(r"\bcargo\s+\+\S")


def scoped_packages(cmd: str) -> set[str] | None:
    """Packages a cargo command touches. None means 'the whole workspace'."""
    explicit = set(re.findall(r"(?:-p|--package)[= ]+([A-Za-z0-9_-]+)", cmd))
    if explicit:
        return explicit
    if "--workspace" in cmd or "--all " in cmd or cmd.rstrip().endswith("--all"):
        excluded = set(re.findall(r"--exclude[= ]+([A-Za-z0-9_-]+)", cmd))
        return None if not excluded else {"\0workspace"} | excluded
    # No package selector: cargo operates on the current directory's package.
    # Every such invocation in this workflow runs from the workspace root,
    # which is virtual, so cargo would error rather than silently pick one.
    return set()


def leaks(cmd: str) -> set[str]:
    """Floor crates this command would build with all features, if any."""
    if "--all-features" not in cmd:
        return set()
    if PINNED_TOOLCHAIN.search(cmd):
        return set()

    scope = scoped_packages(cmd)
    if scope is None:
        return set(FLOOR_CRATES)
    if "\0workspace" in scope:
        return FLOOR_CRATES - scope
    return FLOOR_CRATES & scope


def main() -> int:
    # Scanned as text, not parsed as YAML, so the check has no third-party
    # dependency and runs identically on a bare runner and a workstation.
    # Every cargo invocation in this workflow is one line of a `run:` block,
    # and the nearest preceding `- name:` identifies the step.
    path = sys.argv[1] if len(sys.argv) > 1 else ".github/workflows/ci.yml"
    with open(path) as fh:
        lines = fh.read().splitlines()

    problems: list[str] = []
    step = "<unnamed step>"
    for lineno, line in enumerate(lines, start=1):
        named = re.match(r"\s*-?\s*name:\s*(.+)", line)
        if named:
            step = named.group(1).strip()
            continue
        if "cargo" not in line:
            continue
        hit = leaks(line)
        if hit:
            problems.append(
                f"{path}:{lineno}  ({step})\n"
                f"    {line.strip()}\n"
                f"    reaches {sorted(hit)} with --all-features on the MSRV toolchain"
            )

    if problems:
        print("MSRV containment leak — --all-features reaches a 1.88-floor crate:\n")
        for p in problems:
            print(f"  {p}\n")
        print(
            "Fix by one of: exclude the crate (--exclude), scope the command to\n"
            "packages that do not reach it, or pin a newer toolchain (cargo +1.97.1)\n"
            "and cover the MSRV half in the dedicated split job."
        )
        return 1

    print(f"MSRV containment OK — no --all-features invocation reaches {sorted(FLOOR_CRATES)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
