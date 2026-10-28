#!/usr/bin/env python3
"""Guard for dangling documentation references.

Scans:
  * docs/**/*.md
  * crate README.md files (*/README.md, README.md)
  * rustdoc line comments (//, ///, //!) in *.rs

Checks (exit 1 on any finding):
  1. relative Markdown links / backticked relative repo paths that do not resolve;
  2. citations of docs/... paths that do not exist on disk and are not decoded by
     docs/specs/PRIVATE-MAP.md.
"""

import os
import posixpath
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MAP = ROOT / "docs" / "specs" / "PRIVATE-MAP.md"

MD_LINK = re.compile(r"\[[^\]]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)|^\[[^\]]+\]:\s*(\S+)", re.M)
BACKTICK = re.compile(r"`([^`\n]+)`")
CITE = re.compile(r"(?<![\w./-])((?:[\w.+-]+/)*docs/[\w.+/-]*[\w-]\.(?:md|html|tsv))")
MAP_ROW = re.compile(r"^\| `([^`]+)` \| `([^`]+)` \|\s*(.*?)\s*\|$")


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


def load_map() -> set[str]:
    olds: set[str] = set()
    if not MAP.is_file():
        return olds
    for line in MAP.read_text(encoding="utf-8").splitlines():
        m = MAP_ROW.match(line)
        if m:
            olds.add(m.group(1))
    return olds


def in_map(cite: str, olds: set[str]) -> bool:
    if cite in olds:
        return True
    for o in olds:
        if o == cite or o.endswith("/" + cite):
            return True
    # tail decode: crate-relative docs/... tail
    for o in olds:
        m = re.match(r"^[^/]+/(docs/.*)$", o)
        if m and m.group(1) == cite:
            return True
    return False


def scan_files() -> list[Path]:
    files: list[Path] = []
    docs_dir = ROOT / "docs"
    if docs_dir.is_dir():
        for p in docs_dir.rglob("*.md"):
            if ".git" in p.parts:
                continue
            files.append(p)
    if (ROOT / "README.md").is_file():
        files.append(ROOT / "README.md")
    for p in ROOT.glob("*/README.md"):
        files.append(p)
    # nested crate READMEs
    for p in ROOT.rglob("README.md"):
        if ".git" in p.parts or "private" in p.parts:
            continue
        if p not in files:
            files.append(p)
    return files


def crate_root(path: Path) -> Path:
    cur = path.parent
    while cur != ROOT and cur != cur.parent:
        if (cur / "Cargo.toml").is_file():
            return cur
        cur = cur.parent
    return ROOT


def resolve_candidates(f: Path, token: str) -> bool:
    token = token.strip()
    cands = [f.parent / token]
    if (ROOT / token).exists():
        cands.append(ROOT / token)
    cr = crate_root(f)
    cands.append(cr / token)
    for c in cands:
        try:
            if c.exists():
                return True
        except OSError:
            pass
    return False


def is_relative_path(tok: str) -> bool:
    return ("./" in tok) or ("../" in tok)


def check_markdown(f: Path, olds: set[str], errors: list[str]) -> None:
    text = f.read_text(encoding="utf-8", errors="replace")
    rel = str(f.relative_to(ROOT))
    text_no_code = re.sub(r"`[^`]*`", "", text)
    # (a) relative markdown links
    for m in MD_LINK.finditer(text_no_code):
        tgt = m.group(1) or m.group(2)
        if not tgt:
            continue
        if re.match(r"^[a-z][a-z0-9+.-]*:|^#|^/", tgt):
            continue
        path = tgt.split("#")[0].split("?")[0]
        if not path:
            continue
        resolved = (f.parent / path)
        try:
            resolved = resolved.resolve()
        except OSError:
            pass
        if not resolved.exists():
            errors.append(f"{rel}: relative Markdown link does not resolve: {tgt}")
    # (a) backticked relative repo paths
    for m in BACKTICK.finditer(text):
        tok = m.group(1).strip()
        if not tok or " " in tok:
            continue
        if tok.startswith("http") or tok.startswith("#"):
            continue
        if "*" in tok or "<" in tok or ">" in tok or "{" in tok or "..." in tok:
            continue
        if not re.fullmatch(r"[./A-Za-z0-9_-]+", tok):
            continue
        if not is_relative_path(tok):
            continue
        tt = tok.rstrip(".,;:)")
        if not resolve_candidates(f, tt):
            errors.append(f"{rel}: backticked relative path does not resolve: {tok}")
    # (b) citations of docs/... paths
    for m in CITE.finditer(text):
        cite = m.group(1)
        if cite.startswith("private/"):
            continue
        if "release-notes/" in cite:
            continue
        crate = rel.split("/")[0]
        d = posixpath.dirname(rel)
        cands = [cite, f"{crate}/{cite}", posixpath.normpath(posixpath.join(d, cite))]
        if any((ROOT / c).exists() for c in cands):
            continue
        if in_map(cite, olds) or any(in_map(c, olds) for c in cands):
            continue
        errors.append(f"{rel}: citation does not resolve and is not in docs/specs/PRIVATE-MAP.md: {cite}")


def check_rust(f: Path, olds: set[str], errors: list[str]) -> None:
    text = f.read_text(encoding="utf-8", errors="replace")
    rel = str(f.relative_to(ROOT))
    cr = crate_root(f)
    for line in text.splitlines():
        s = line.strip()
        if not s.startswith("//"):
            continue
        # (a) relative markdown links in comments
        line_no_code = re.sub(r"`[^`]*`", "", line)
        for m in MD_LINK.finditer(line_no_code):
            tgt = m.group(1) or m.group(2)
            if not tgt:
                continue
            if re.match(r"^[a-z][a-z0-9+.-]*:|^#|^/", tgt):
                continue
            path = tgt.split("#")[0].split("?")[0]
            if not path:
                continue
            if "::" in path:
                continue
            base = path.split("/")[-1]
            if "/" not in path and "." not in base:
                continue
            resolved = (f.parent / path)
            try:
                resolved = resolved.resolve()
            except OSError:
                pass
            if not resolved.exists():
                errors.append(f"{rel}: relative Markdown link does not resolve: {tgt}")
        # (a) backticked relative repo paths in comments
        for m in BACKTICK.finditer(line):
            tok = m.group(1).strip()
            if not tok or " " in tok:
                continue
            if tok.startswith("http") or tok.startswith("#"):
                continue
            if "*" in tok or "<" in tok or ">" in tok or "{" in tok or "..." in tok:
                continue
            if not re.fullmatch(r"[./A-Za-z0-9_-]+", tok):
                continue
            if not is_relative_path(tok):
                continue
            tt = tok.rstrip(".,;:)")
            cands = [f.parent / tt, ROOT / tt, cr / tt]
            if any(c.exists() for c in cands):
                continue
            errors.append(f"{rel}: backticked relative path does not resolve: {tok}")
        # (b) citations of docs/... paths in comments
        for m in CITE.finditer(line):
            cite = m.group(1)
            if cite.startswith("private/"):
                continue
            if "release-notes/" in cite:
                continue
            d = posixpath.dirname(rel)
            cands = [cite, f"{rel.split('/')[0]}/{cite}", posixpath.normpath(posixpath.join(d, cite))]
            if any((ROOT / c).exists() for c in cands):
                continue
            if in_map(cite, olds) or any(in_map(c, olds) for c in cands):
                continue
            errors.append(f"{rel}: citation does not resolve and is not in docs/specs/PRIVATE-MAP.md: {cite}")


def main() -> int:
    olds = load_map()
    errors: list[str] = []
    for f in scan_files():
        try:
            check_markdown(f, olds, errors)
        except (OSError, UnicodeDecodeError):
            continue
    for f in ROOT.rglob("*.rs"):
        if ".git" in f.parts or "private" in f.parts:
            continue
        try:
            check_rust(f, olds, errors)
        except (OSError, UnicodeDecodeError):
            continue
    if errors:
        print("check-doc-links: FAILED", file=sys.stderr)
        for e in errors:
            print("  " + e, file=sys.stderr)
        return 1
    print(f"check-doc-links: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
