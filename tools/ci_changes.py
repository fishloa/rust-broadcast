#!/usr/bin/env python3
"""Classify a change set for ci.yml's `changes` job.

Reads changed paths (one per line) on stdin and prints `rust=true` or
`rust=false`. `rust=false` only when EVERY path is Markdown under `docs/` or a
non-README `.md` file: no Rust job can be affected by those. A crate README is
`include_str!`-ed into its crate docs (a doctest), so any README.md counts as
Rust-relevant. An empty change set is Rust-relevant (fail safe).
"""

import posixpath
import sys


def is_docs_only(path: str) -> bool:
    path = path.strip()
    if not path.endswith(".md"):
        return False
    if posixpath.basename(path) == "README.md":
        return False
    return True


def rust_relevant(paths: list[str]) -> bool:
    paths = [p for p in (p.strip() for p in paths) if p]
    if not paths:
        return True
    return not all(is_docs_only(p) for p in paths)


def main() -> int:
    print(f"rust={'true' if rust_relevant(sys.stdin.read().splitlines()) else 'false'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
