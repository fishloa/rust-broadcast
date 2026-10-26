#!/usr/bin/env python3
"""Guard for the private spec move (docs/specs/PRIVATE-MAP.md).

Spec transcriptions live in the private companion repo; the public tree keeps a
table (old public path -> private path) so historical citations stay decodable.

Checks (exit 1 on any failure):
  1. every map row's old path is absent from the public tree, unless the row is
     marked "split" (the public file then keeps only its own-words part);
  2. no Markdown link (outside CHANGELOG / release notes / superpowers) targets a
     moved path or a directory that only held moved files;
  3. every `docs/...` citation in tracked text files resolves to an existing file,
     or to a map row (by full path or crate-relative tail), unless it is listed
     in tools/private-map-baseline.txt (citations that were already dangling
     before the move; the list may only shrink);
  4. with `private/specs-verified` present, every map row's private target exists.
"""

import posixpath
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MAP = ROOT / "docs" / "specs" / "PRIVATE-MAP.md"
BASELINE = ROOT / "tools" / "private-map-baseline.txt"
PRIVATE_PREFIX = "private/specs-verified/"

HISTORICAL = re.compile(r"(^|/)CHANGELOG[^/]*\.md$|^docs/release-notes/|^docs/superpowers/|^\.delegate/|^private/|^tools/private-map-baseline\.txt$|^docs/specs/PRIVATE-MAP\.md$")
LINK = re.compile(r"\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)|^\[[^\]]+\]:\s*(\S+)", re.M)
CITE = re.compile(r"(?<![\w./-])((?:[\w.+-]+/)*docs/[\w.+/-]*[\w-]\.(?:md|html|tsv))")
TEXT_SUFFIXES = (".md", ".rs", ".toml", ".py", ".txt", ".yml")


def tracked() -> list[str]:
    out = subprocess.run(
        ["git", "-c", "core.quotepath=false", "ls-files"], cwd=ROOT, capture_output=True, text=True, check=True
    ).stdout
    return out.splitlines()


def load_map() -> list[tuple[str, str, str]]:
    rows = []
    for line in MAP.read_text(encoding="utf-8").splitlines():
        m = re.match(r"^\| `([^`]+)` \| `([^`]+)` \|\s*(.*?)\s*\|$", line)
        if m:
            rows.append((m.group(1), m.group(2), m.group(3)))
    return rows


def main() -> int:
    rows = load_map()
    if not rows:
        print("check-private-map: no rows parsed from docs/specs/PRIVATE-MAP.md", file=sys.stderr)
        return 1
    files = tracked()
    present = set(files)
    errors: list[str] = []

    old_paths = {o for o, _, _ in rows}
    split = {o for o, _, n in rows if n.startswith("split")}
    for o in sorted(old_paths):
        if o in present and o not in split:
            errors.append(f"map row {o}: old path still exists in the public tree")

    moved_dirs = {posixpath.dirname(o) for o in old_paths}
    for d in list(moved_dirs):
        while d and d not in moved_dirs:
            moved_dirs.add(d)
            d = posixpath.dirname(d)

    def is_moved(res: str) -> bool:
        res = posixpath.normpath(res)
        if res in old_paths and res not in present:
            return True
        if res in moved_dirs and not any(p.startswith(res + "/") for p in present):
            return True
        return False

    texts = [f for f in files if f.endswith(TEXT_SUFFIXES) and not HISTORICAL.search(f)]
    docs_cache: dict[str, str] = {}
    for f in texts:
        try:
            docs_cache[f] = (ROOT / f).read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue

    for f, s in docs_cache.items():
        if not f.endswith(".md"):
            continue
        d = posixpath.dirname(f)
        for m in LINK.finditer(s):
            tgt = m.group(1) or m.group(2)
            if not tgt or re.match(r"^[a-z][a-z0-9+.-]*:|^#|^/", tgt):
                continue
            path = tgt.split("#")[0].split("?")[0]
            if path and is_moved(posixpath.join(d, path)):
                errors.append(f"{f}: Markdown link to moved path: {tgt}")

    tails = {}
    for o in old_paths:
        m = re.match(r"^[^/]+/(docs/.*)$", o)
        if m:
            tails.setdefault(m.group(1), []).append(o)
    baseline = set()
    if BASELINE.exists():
        baseline = {l.strip() for l in BASELINE.read_text().splitlines() if l.strip() and not l.startswith("#")}
    seen_baseline = set()
    reported: set[str] = set()
    for f, s in docs_cache.items():
        crate = f.split("/")[0]
        d = posixpath.dirname(f)
        for m in CITE.finditer(s):
            cite = m.group(1)
            if cite.startswith("private/") or "release-notes/" in cite:
                continue
            cands = [cite, f"{crate}/{cite}", posixpath.normpath(posixpath.join(d, cite))]
            if any(c in present or c in old_paths for c in cands):
                continue
            if cite in tails and any(o.startswith(crate + "/") for o in tails[cite]):
                continue
            key = f"{f}\t{cite}"
            if key in baseline:
                seen_baseline.add(key)
                continue
            if key not in reported:
                reported.add(key)
                errors.append(f"{f}: citation does not resolve and is not in docs/specs/PRIVATE-MAP.md: {cite}")

    stale = baseline - seen_baseline
    for key in sorted(stale):
        errors.append(f"tools/private-map-baseline.txt: entry no longer needed, delete it: {key}")

    private_root = ROOT / "private" / "specs-verified"
    if private_root.is_dir():
        for o, n, _ in rows:
            if not (ROOT / n).is_file():
                errors.append(f"map row {o}: private target missing: {n}")
    else:
        print("check-private-map: private/specs-verified not checked out; skipping private-target existence check")

    if errors:
        print("check-private-map: FAILED", file=sys.stderr)
        for e in errors:
            print("  " + e, file=sys.stderr)
        return 1
    print(f"check-private-map: ok ({len(rows)} rows)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
