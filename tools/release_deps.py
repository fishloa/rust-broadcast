#!/usr/bin/env python3
"""Small, testable helpers behind `tools/push-release-tags.sh`.

    release_deps.py version                 < Cargo.toml   -> the package version
    release_deps.py deps [--skip a,b]       < Cargo.toml   -> "<crate> <req>" lines
    release_deps.py max                     < index body   -> max non-yanked semver
    release_deps.py satisfies <req>         < index body   -> "OK <ver>" | "NO <vers>"
    release_deps.py note-ok                 < release note -> exit 0 unless DRAFT/empty

`deps` lists every WORKSPACE dependency of the manifest on stdin at the version
REQUIREMENT the manifest carries: an entry with a `path` and a `version` in
[dependencies], [build-dependencies], [target.*.*] tables, plus VERSIONED
[dev-dependencies] (cargo resolves those against the index at publish time; a
path-only dev-dependency is stripped and gates nothing).

`max` / `satisfies` read the crates.io sparse-index body (one JSON object per
line) and take the MAX SEMVER, never the last line: index order is publish
order, and a patch release of an older line (`9.0.1` after `10.0.0`) is the last
line without being the newest version. Yanked versions never satisfy anything.
"""

from __future__ import annotations

import json
import re
import sys
import tomllib

_SEMVER = re.compile(r"^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$")


def parse_version(v: str) -> tuple:
    m = _SEMVER.match(v.strip())
    if not m:
        raise ValueError(f"not a semver version: {v!r}")
    pre = m.group(4)
    ids: tuple = ()
    if pre:
        ids = tuple((0, int(p), "") if p.isdigit() else (1, 0, p) for p in pre.split("."))
    # a release (no pre-release) sorts ABOVE any pre-release of the same triple
    return (int(m.group(1)), int(m.group(2)), int(m.group(3)), 1 if not pre else 0, ids)


def _bounds(req: str) -> tuple[tuple[int, int, int], tuple[int, int, int] | None]:
    """[lo, hi) for one comparator (caret default, `~`, `=`, bare)."""
    r = req.strip()
    op = "^"
    for cand in ("^", "~", "="):
        if r.startswith(cand):
            op, r = cand, r[1:].strip()
            break
    if not re.fullmatch(r"\d+(\.\d+){0,2}", r):
        raise ValueError(f"unsupported version requirement {req!r}")
    parts = [int(x) for x in r.split(".")]
    n = len(parts)
    parts += [0] * (3 - n)
    lo = (parts[0], parts[1], parts[2])
    if op == "=":
        if n == 3:
            return lo, (lo[0], lo[1], lo[2] + 1)
        if n == 2:
            return lo, (lo[0], lo[1] + 1, 0)
        return lo, (lo[0] + 1, 0, 0)
    if op == "~":
        return lo, ((lo[0], lo[1] + 1, 0) if n >= 2 else (lo[0] + 1, 0, 0))
    # caret
    if lo[0] > 0 or n == 1:
        return lo, (lo[0] + 1, 0, 0)
    if lo[1] > 0 or n == 2:
        return lo, (0, lo[1] + 1, 0)
    return lo, (0, 0, lo[2] + 1)


def satisfies(version: str, req: str) -> bool:
    key = parse_version(version)
    if key[4] or key[3] == 0:  # pre-releases never satisfy a plain requirement
        return False
    lo, hi = _bounds(req)
    triple = key[:3]
    return triple >= lo and (hi is None or triple < hi)


def index_versions(body: str) -> list[tuple[str, bool]]:
    out: list[tuple[str, bool]] = []
    for line in body.splitlines():
        if line.strip():
            d = json.loads(line)
            out.append((d["vers"], bool(d.get("yanked", False))))
    return out


def max_version(body: str) -> str | None:
    live = [v for v, y in index_versions(body) if not y]
    return max(live, key=parse_version) if live else None


def workspace_deps(manifest_text: str, skip: set[str] | None = None) -> list[tuple[str, str]]:
    data = tomllib.loads(manifest_text)
    skip = skip or set()
    own = (data.get("package") or {}).get("name")
    found: dict[str, str] = {}

    def take(table: dict) -> None:
        for kind in ("dependencies", "build-dependencies", "dev-dependencies"):
            for name, spec in (table.get(kind) or {}).items():
                if not isinstance(spec, dict) or "path" not in spec or "version" not in spec:
                    continue  # third-party, or path-only (stripped at publish)
                real = spec.get("package", name)
                if real != own and real not in skip:
                    found[real] = spec["version"]

    take(data)
    for tgt in (data.get("target") or {}).values():
        take(tgt)
    return sorted(found.items())


def package_version(manifest_text: str) -> str:
    return tomllib.loads(manifest_text)["package"]["version"]


def note_ok(text: str) -> tuple[bool, str]:
    if not text.strip():
        return False, "empty"
    m = re.match(r"\A---[ \t]*\n(.*?)\n---[ \t]*(?:\n|\Z)", text, re.S)
    if m and re.search(r"^status:\s*DRAFT\s*$", m.group(1), re.M):
        return False, "still `status: DRAFT`"
    return True, ""


def main(argv: list[str]) -> int:
    if len(argv) < 2:
        print(__doc__, file=sys.stderr)
        return 64
    cmd, rest = argv[1], argv[2:]
    data = sys.stdin.read()
    try:
        if cmd == "version":
            print(package_version(data))
        elif cmd == "deps":
            skip = set(rest[1].split(",")) if len(rest) == 2 and rest[0] == "--skip" else set()
            for name, req in workspace_deps(data, skip):
                print(f"{name} {req}")
        elif cmd == "max":
            print(max_version(data) or "UNPUBLISHED")
        elif cmd == "satisfies":
            req = rest[0]
            live = [v for v, y in index_versions(data) if not y]
            ok = sorted((v for v in live if satisfies(v, req)), key=parse_version)
            if ok:
                print(f"OK {ok[-1]}")
                return 0
            print("NO " + (",".join(sorted(live, key=parse_version)) or "none-live"))
            return 1
        elif cmd == "note-ok":
            ok, why = note_ok(data)
            if not ok:
                print(why)
            return 0 if ok else 1
        else:
            print(f"unknown command {cmd}", file=sys.stderr)
            return 64
    except (ValueError, KeyError, json.JSONDecodeError, tomllib.TOMLDecodeError) as e:
        print(f"ERR {e}")
        return 3
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
