#!/usr/bin/env python3
"""Keep README dependency snippets in step with the workspace crates' versions.

A README line such as

    dvb-si = "11"
    mpeg-ts = { version = "0.5", default-features = false }

names a workspace crate and a version requirement. Those lines went stale every
wave (13 crates before this tool existed) because nothing derived them from the
bump plan. This module owns that derivation:

* `find_snippets(root, versions)` -- every such line in every `README.md`.
* `expected_req(req, version)` -- the requirement the snippet SHOULD say for
  `version`, preserving how many components the author wrote (`"11"` stays a
  major-only requirement, `"0.5"` stays major.minor, `"0.5.1"` stays full); a
  0.x requirement is never shortened below major.minor, as `"0"` would span
  every epoch.
* `check(root, versions)` -- the snippets that disagree.
* `rewrite(root, versions)` -- rewrite them in place.

`versions` maps crate -> version string: the crate's current manifest version,
overlaid by the bump plan when one is given. Only workspace crates are touched;
`path`-only or `git` dependencies without a version are ignored.

CLI (also driven from tools/prepare-release.py's apply and --check paths):

    tools/readme_snippets.py --check [crate=version ...]   # exit 1 on any stale snippet
    tools/readme_snippets.py --apply [crate=version ...]   # rewrite in place
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

VERSION_RE = re.compile(r'^version\s*=\s*"([^"]+)"', re.M)
NAME_RE = re.compile(r'^name\s*=\s*"([^"]+)"', re.M)

# `name = "req"` or `name = { ... version = "req" ... }` at the start of a line.
SNIPPET_RE = re.compile(
    r'^(?P<head>[ \t]*(?P<name>[A-Za-z0-9_-]+)[ \t]*=[ \t]*'
    r'(?:\{[^}\n]*?\bversion[ \t]*=[ \t]*)?)"(?P<req>[0-9][0-9A-Za-z.+-]*)"',
    re.M,
)

SKIP_DIRS = {"target", "node_modules", "private", ".git", ".claude", "fuzz", "demo"}


def workspace_versions(root: Path = ROOT) -> dict[str, str]:
    """crate name -> current manifest version, for every `<dir>/Cargo.toml`."""
    out: dict[str, str] = {}
    for manifest in sorted(root.glob("*/Cargo.toml")):
        text = manifest.read_text()
        n, v = NAME_RE.search(text), VERSION_RE.search(text)
        if n and v:
            out[n.group(1)] = v.group(1)
    return out


def readmes(root: Path = ROOT) -> list[Path]:
    found = [root / "README.md"] if (root / "README.md").is_file() else []
    for p in sorted(root.glob("*/README.md")):
        if p.parent.name not in SKIP_DIRS:
            found.append(p)
    return found


def expected_req(req: str, version: str) -> str:
    """The requirement `req` should be, given the crate is at `version`."""
    have = len(req.split("."))
    parts = version.split(".")
    major_zero = parts[0] == "0"
    n = min(max(have, 2 if major_zero else 1), 3)
    return ".".join(parts[:n])


def find_snippets(root: Path, versions: dict[str, str]):
    """Yield (path, line_no, crate, req, expected) for every workspace snippet."""
    for path in readmes(root):
        text = path.read_text()
        for m in SNIPPET_RE.finditer(text):
            name = m.group("name")
            if name not in versions:
                continue
            line = text.count("\n", 0, m.start()) + 1
            yield path, line, name, m.group("req"), expected_req(m.group("req"), versions[name])


def check(root: Path, versions: dict[str, str]) -> list[str]:
    return [
        f'{p.relative_to(root)}:{ln}: {name} = "{req}" should be "{want}"'
        for p, ln, name, req, want in find_snippets(root, versions)
        if req != want
    ]


def rewrite(root: Path, versions: dict[str, str], apply: bool = True) -> list[str]:
    """Rewrite stale snippets; returns one description per changed line."""
    changes: list[str] = []
    for path in readmes(root):
        text = path.read_text()

        def sub(m: re.Match[str]) -> str:
            name = m.group("name")
            if name not in versions:
                return m.group(0)
            want = expected_req(m.group("req"), versions[name])
            if want == m.group("req"):
                return m.group(0)
            line = text.count("\n", 0, m.start()) + 1
            changes.append(f'  {path.relative_to(root)}:{line}: {name} "{m.group("req")}" -> "{want}"')
            return f'{m.group("head")}"{want}"'

        new = SNIPPET_RE.sub(sub, text)
        if apply and new != text:
            path.write_text(new)
    return changes


def plan_versions(pairs: list[tuple[str, str]], root: Path = ROOT) -> dict[str, str]:
    versions = workspace_versions(root)
    versions.update(dict(pairs))
    return versions


def main(argv: list[str]) -> int:
    if not argv or argv[0] not in ("--check", "--apply"):
        print(__doc__, file=sys.stderr)
        return 2
    pairs: list[tuple[str, str]] = []
    for a in argv[1:]:
        if "=" not in a:
            raise SystemExit(f"bad argument {a!r}, expected <crate>=<version>")
        c, v = a.split("=", 1)
        pairs.append((c, v))
    versions = plan_versions(pairs)
    if argv[0] == "--apply":
        for line in rewrite(ROOT, versions) or ["  (none needed)"]:
            print(line)
        return 0
    bad = check(ROOT, versions)
    for line in bad:
        print(line)
    if bad:
        print(f"\n{len(bad)} stale README dependency snippet(s); run `tools/readme_snippets.py --apply`.", file=sys.stderr)
        return 1
    print("README dependency snippets OK")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
