#!/usr/bin/env python3
"""Guard for spec citations after the private move.

Scans every tracked text file outside private/ and fails when:
  a. a cited relative path ending in a doc suffix and starting with docs/,
     specs/, fixtures/ or <crate>/docs/ does not exist in the public tree;
  b. a cited private/... path does not exist (when private/specs-verified
     is checked out);
  c. a Markdown link target points into private/.
"""

import fnmatch
import os
import posixpath
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

DOC_SUFFIXES = (".md", ".txt", ".pdf", ".tsv", ".toml", ".html")
MD_LINK = re.compile(r"\[[^\]]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)|^\[[^\]]+\]:\s*(\S+)", re.M)
# citations: path containing docs/specs/fixtures root, ending in a doc suffix
CITE = re.compile(
    r"(?<![\w./-])((?:[\w.+-]+/)*?(?:docs|specs|fixtures)(?:/[\w.+*?\[\]-]+)*\."
    r"(?:md|txt|pdf|tsv|toml|html))"
)
PRIVATE_CITE = re.compile(
    r"(?<![\w./-])(private/(?:[\w.+\-*?\[\]]+/)*[\w.+\-*?\[\]]+\."
    r"(?:md|txt|pdf|tsv|toml|html))"
)
GITHUB_URL = re.compile(
    r"https://github\.com/fishloa/rust-broadcast/(?:blob|tree)/[^/]+/([^\s)\"#?]+)"
)
GLOB_CHARS = set("*?[")
HISTORICAL = re.compile(r"(^|/)CHANGELOG[^/]*\.md$|^docs/release-notes/|^docs/superpowers/|^\.delegate/|^private/")



def rust_string_spans(s):
    spans = []
    i = 0
    n = len(s)
    while i < n:
        if s.startswith("//", i):
            j = s.find("\n", i)
            if j == -1:
                j = n
            i = j
            continue
        if s.startswith("/*", i):
            j = s.find("*/", i)
            if j == -1:
                j = n
            else:
                j += 2
            i = j
            continue
        if s[i] == '"':
            hashcount = 0
            raw = False
            if i > 0 and s[i - 1] == "r":
                k = i + 1
                while k < n and s[k] == "#":
                    hashcount += 1
                    k += 1
                if k < n and s[k] == '"':
                    raw = True
            if raw:
                k = i + 1 + hashcount
                end = None
                while True:
                    idx = s.find('"', k)
                    if idx == -1:
                        end = n
                        break
                    h = 0
                    while idx + 1 + h < n and s[idx + 1 + h] == "#":
                        h += 1
                    if h == hashcount:
                        end = idx + 1 + h
                        break
                    k = idx + 1
                spans.append((i, end))
                i = end
                continue
            else:
                j = i + 1
                while j < n:
                    if s[j] == "\\":
                        j += 2
                        continue
                    if s[j] == '"':
                        break
                    if s[j] == "\n":
                        break
                    j += 1
                if j >= n:
                    j = n
                else:
                    j += 1
                spans.append((i, j))
                i = j
                continue
        elif s[i] == "'":
            j = i + 1
            if j < n and s[j] == "\\":
                j += 2
            else:
                j += 1
            if j < n and s[j] == "'":
                j += 1
            i = j
            continue
        else:
            i += 1
    return spans


def py_string_spans(s):
    spans = []
    i = 0
    n = len(s)
    while i < n:
        c = s[i]
        if c == "#":
            j = s.find("\n", i)
            if j == -1:
                j = n
            i = j
            continue
        if c in ('"', "'"):
            triple = s.startswith(c * 3, i)
            if triple:
                quote = c * 3
                j = s.find(quote, i + 3)
                if j == -1:
                    j = n
                else:
                    j += 3
                spans.append((i, j))
                i = j
                continue
            else:
                q = c
                j = i + 1
                while j < n:
                    if s[j] == "\\":
                        j += 2
                        continue
                    if s[j] == q:
                        break
                    if s[j] == "\n":
                        break
                    j += 1
                if j >= n:
                    j = n
                else:
                    j += 1
                spans.append((i, j))
                i = j
                continue
        else:
            i += 1
    return spans


def in_spans(pos, spans):
    for a, b in spans:
        if a <= pos < b:
            return True
    return False

def tracked() -> list[str]:
    try:
        out = subprocess.run(
            ["git", "-c", "core.quotepath=false", "ls-files"], cwd=ROOT,
            capture_output=True, text=True, check=True,
        ).stdout
        return out.splitlines()
    except Exception:
        result = []
        for p in ROOT.rglob("*"):
            if ".git" in p.parts:
                continue
            if p.is_file():
                result.append(str(p.relative_to(ROOT)))
        return result


def is_text(path: Path) -> bool:
    try:
        data = path.read_bytes()
    except OSError:
        return False
    if b"\x00" in data:
        return False
    try:
        data.decode("utf-8")
    except UnicodeDecodeError:
        return False
    return True


def has_prefix(p: str) -> bool:
    if p.startswith("docs/") or p.startswith("specs/") or p.startswith("fixtures/"):
        return True
    # <crate>/docs/
    parts = p.split("/")
    return len(parts) >= 3 and parts[1] == "docs"


def main() -> int:
    files = tracked()
    present = set(f for f in files if not f.startswith("private/"))
    # also private gitlink? exclude
    present = set(f for f in present if f != "private")

    private_root = ROOT / "private" / "specs-verified"
    if not private_root.is_dir():
        if os.environ.get("REQUIRE_PRIVATE_FIXTURES") == "1":
            print("check-citations: FAILED", file=sys.stderr)
            print("  private/specs-verified not checked out (REQUIRE_PRIVATE_FIXTURES=1)", file=sys.stderr)
            return 1
        skip_private = True
        print("check-citations: private/specs-verified not checked out; skipping private-path existence check")
    else:
        skip_private = False

    errors: list[str] = []

    # private file list for glob checks
    private_files = []
    if private_root.is_dir():
        for root, _, fs in os.walk(ROOT / "private"):
            for fn in fs:
                private_files.append(os.path.join(root, fn).replace(os.sep, "/").replace(str(ROOT) + "/", ""))

    for rel in files:
        if rel.startswith("private/"):
            continue
        if rel == "private":
            continue
        if HISTORICAL.search(rel):
            continue
        if os.path.basename(rel) == ".gitignore":
            continue
        p = ROOT / rel
        if not p.is_file() or not is_text(p):
            continue
        try:
            text = p.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        crate = rel.split("/")[0]
        d = posixpath.dirname(rel)
        if rel.endswith(".rs"):
            spans = rust_string_spans(text)
        elif rel.endswith(".py"):
            spans = py_string_spans(text)
        else:
            spans = []

        # a. public citations
        for m in CITE.finditer(text):
            cite = m.group(1)
            if not has_prefix(cite):
                continue
            if "release-notes/" in cite:
                continue
            if spans and in_spans(m.start(1), spans):
                continue
            # glob?
            if any(ch in cite for ch in GLOB_CHARS):
                if "**" in cite:
                    continue
                pats = [cite, f"{crate}/{cite}", posixpath.normpath(posixpath.join(d, cite))]
                matched = False
                for pat in pats:
                    for cand in present:
                        if fnmatch.fnmatch(cand, pat):
                            matched = True
                            break
                    if matched:
                        break
                if not matched:
                    errors.append(f"{rel}: citation does not resolve: {cite}")
            else:
                cands = [cite, f"{crate}/{cite}", posixpath.normpath(posixpath.join(d, cite))]
                if not any(c in present for c in cands):
                    if not any((ROOT / c).is_file() for c in cands):
                        errors.append(f"{rel}: citation does not resolve: {cite}")

        # b. private citations
        if not skip_private:
            for m in PRIVATE_CITE.finditer(text):
                cite = m.group(1)
                if spans and in_spans(m.start(1), spans):
                    continue
                if any(ch in cite for ch in GLOB_CHARS):
                    if "**" in cite:
                        continue
                    if not any(fnmatch.fnmatch(f, cite) for f in private_files):
                        errors.append(f"{rel}: private citation does not exist: {cite}")
                else:
                    if not (ROOT / cite).is_file():
                        errors.append(f"{rel}: private citation does not exist: {cite}")

        # c. markdown links into private/
        for m in MD_LINK.finditer(text):
            tgt = m.group(1) or m.group(2)
            if not tgt:
                continue
            path = tgt.split("#")[0].split("?")[0]
            if not path:
                continue
            if spans and in_spans(m.start(0), spans):
                continue
            resolved = posixpath.normpath(posixpath.join(d, path))
            if resolved == "private" or resolved.startswith("private/") or path.startswith("private/"):
                errors.append(f"{rel}: Markdown link into private/: {tgt}")

        # d. absolute github URLs with nonexistent public paths
        for m in GITHUB_URL.finditer(text):
            path = m.group(1)
            if not (ROOT / path).exists():
                errors.append(f"{rel}: GitHub URL path does not exist: {path}")

    if errors:
        print("check-citations: FAILED", file=sys.stderr)
        for e in errors:
            print("  " + e, file=sys.stderr)
        return 1
    print("check-citations: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
