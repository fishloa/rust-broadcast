#!/usr/bin/env python3
"""Guard for spec citations after the private move.

Scans every tracked text file outside private/ and fails when:
  a. a cited relative path ending in a doc suffix and starting with docs/,
     specs/, fixtures/ or <crate>/docs/ does not exist in the public tree;
  b. a cited private/... path does not exist (when private/specs-verified
     is checked out);
  c. a Markdown link target points into private/;
  g. a retired location is named: a `tests/fixtures` or `tests/golden` path
     (fixtures and goldens live under fixtures/<topic>/ and
     fixtures/golden/<topic>/), `docs/specs/`, `PRIVATE-MAP`, or
     `.test-streams` outside the fetch script;
  h. a `../fixtures/...` citation, or a literal `test_fixtures::path|read|
     read_to_string("<topic>/<file>")` argument, does not resolve;
  j. a bare spec file name (`en_301_192.md`, `ts_102_809_apps.md`, ...) that
     is not the name of any file in the public tree or private/ (when it is
     checked out); cite the private/specs-verified/... path instead;
  i. a `<member>/(src|tests|examples|benches|docs)/...` or bare
     `(src|tests|examples|benches|docs)/...` path to a source, script, doc or
     manifest file named in a comment or document does not exist (string
     literals of .rs/.py files are skipped; those are exercised by tests).
"""

import fnmatch
import os
import posixpath
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def _load_workspace_members():
    members = set()
    cargo = ROOT / "Cargo.toml"
    try:
        text = cargo.read_text(encoding="utf-8")
    except OSError:
        return members
    m = re.search(r"members\s*=\s*\[(.*?)\]", text, re.S)
    if not m:
        return members
    for item in re.findall(r"\"([^\"]+)\"", m.group(1)):
        item = item.strip().rstrip("/")
        if item:
            members.add(item)
    return members



WORKSPACE_MEMBERS = _load_workspace_members()

DOC_SUFFIXES = (".md", ".txt", ".pdf", ".tsv", ".toml", ".html")
MD_LINK = re.compile(r"\[[^\]]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)|^\[[^\]]+\]:\s*(\S+)", re.M)
# fixture citations: fixtures/ or private/fixtures/ with any extension (or a directory)
FIXTURE_CITE = re.compile(
    r"(?<![\w./-])((?:fixtures|private/fixtures)(?:/[\w.+*?\[\]-]*)+/?)"
)
# crate-local fixture citations: <member>/fixtures/... or <member>/tests/fixtures/...
MEMBER_FIXTURE_CITE = re.compile(
    r"(?<![\w./-])(([\w.+-]+)/(?:tests/)?fixtures(?:/[\w.+*?\[\]-]*)*/?)"
)
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
RETIRED_PATH = re.compile(r"(?<![\w.-])(?:[\w.+-]+/)*tests/(?:fixtures|golden)(?:/|(?![\w.-]))")
RETIRED_DOCS_SPECS = re.compile(r"(?<![\w-])docs/specs/")
RETIRED_PRIVATE_MAP = re.compile(r"PRIVATE-MAP")
RETIRED_TEST_STREAMS = re.compile(r"\.test-streams")
RELATIVE_FIXTURE_CITE = re.compile(
    r"(?<![\w./-])((?:\.\./)+fixtures(?:/[\w.+@=*?\[\]{},-]*)*)"
)
FIXTURE_CALL = re.compile(
    r"test_fixtures::(?:path|read|read_to_string)\(\s*&?\"([^\"{}\\]+)\"\s*\)"
)
PATH_CITE = re.compile(
    r"(?<![\w./-])((?:[\w+-]+/)*(?:src|tests|examples|benches|docs)/(?:[\w.+-]+/)*[\w.+-]+"
    r"\.(?:rs|py|md|toml|sh|yml|yaml|json|mpd|csv))"
)
SPEC_FILE_NAME = re.compile(
    r"(?<![\w/.-])((?:en|ts|tr|etsi|ansi|iso|itu|smpte|scte|atsc)_[0-9][\w.-]*\.(?:md|pdf|txt))"
)
ISO15_UNDATED = re.compile(r"(?<![\w/.-])14496-15(?!:(?:2017|2022)(?![\w]))")
CRATE_RELATIVE_ROOTS = ("src", "tests", "examples", "benches", "docs")
EXTERNAL_PATH_DOCS = {"docs/migration-notes-downstream.md"}
GLOB_CHARS = set("*?[")
RETIRED_EXEMPT = {
    "docs/FIXTURES.md",
    "workspace-guards/tests/guards/fixtures_one_home.rs",
    "tools/check-citations.py",
    "tools/tests/test_check_citations.py",
}
TEST_STREAMS_ALLOWED = {"tools/fetch-test-streams.sh"}
FIXTURE_CALL_EXEMPT_PREFIX = ("test-fixtures/",)
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


def cite_resolves(cite: str, bases: list[str], present: set[str], present_dirs: set[str]) -> bool:
    partial = cite[-1] in "-_."
    for base in bases:
        target = posixpath.normpath(posixpath.join(base, cite))
        if any(ch in target for ch in GLOB_CHARS):
            if any(fnmatch.fnmatch(c, target) for c in present):
                return True
        elif partial:
            if any(c.startswith(target) for c in present):
                return True
        elif target in present or target in present_dirs:
            return True
    return False


def has_prefix(p: str) -> bool:
    if p.startswith("private/"):
        return False
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
    present_dirs = {posixpath.dirname(f) for f in present}
    present_dirs |= {posixpath.dirname(d) for d in list(present_dirs)}
    while True:
        parents = {posixpath.dirname(d) for d in present_dirs} - present_dirs
        if not parents:
            break
        present_dirs |= parents

    private_root = ROOT / "private" / "specs-verified"
    if os.environ.get("SKIP_PRIVATE_FIXTURES") == "1":
        skip_private = True
        print("check-citations: private/specs-verified skipped (SKIP_PRIVATE_FIXTURES=1)")
    elif not private_root.is_dir():
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

    known_basenames = {posixpath.basename(f) for f in present} | {
        posixpath.basename(f) for f in private_files
    }

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
            if cite.startswith("private/"):
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

        # e. fixture citations (any extension) under fixtures/ or private/fixtures/
        for m in FIXTURE_CITE.finditer(text):
            cite = m.group(1).rstrip().rstrip(".,;:")
            cite_stripped = cite.rstrip("/")
            if cite_stripped.endswith(DOC_SUFFIXES):
                continue
            base = cite_stripped.rsplit("/", 1)[-1]
            if not (cite.endswith("/") or "." in base):
                continue
            if spans and in_spans(m.start(1), spans):
                continue
            if cite.startswith("private/"):
                if skip_private:
                    continue
                if any(ch in cite for ch in GLOB_CHARS):
                    if "**" in cite:
                        continue
                    if not any(fnmatch.fnmatch(f, cite) for f in private_files):
                        errors.append(f"{rel}: private citation does not exist: {cite}")
                else:
                    target = ROOT / cite
                    if not (target.is_file() or target.is_dir()):
                        errors.append(f"{rel}: private citation does not exist: {cite}")
            else:
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
                    if not matched and not any(posixpath.normpath(c) in present_dirs for c in pats):
                        errors.append(f"{rel}: citation does not resolve: {cite}")
                else:
                    cands = [cite, f"{crate}/{cite}", posixpath.normpath(posixpath.join(d, cite))]
                    if not any(c in present or posixpath.normpath(c) in present_dirs for c in cands):
                        errors.append(f"{rel}: citation does not resolve: {cite}")

        # f. crate-local fixture citations are never allowed
        for m in MEMBER_FIXTURE_CITE.finditer(text):
            cite = m.group(1).rstrip()
            member = m.group(2)
            if member not in WORKSPACE_MEMBERS:
                continue
            cite_stripped = cite.rstrip("/")
            base = cite_stripped.rsplit("/", 1)[-1]
            if not (cite.endswith("/") or "." in base):
                continue
            if spans and in_spans(m.start(1), spans):
                continue
            errors.append(
                f"{rel}: citation points at a crate-local fixtures path; "
                f"cite fixtures/<topic>/... or private/fixtures/<topic>/... instead: {cite}"
            )

        # g. retired locations
        if rel not in RETIRED_EXEMPT:
            for pat, what in (
                (RETIRED_PATH, "a retired tests/fixtures or tests/golden path; use fixtures/<topic>/ or fixtures/golden/<topic>/"),
                (RETIRED_DOCS_SPECS, "the retired docs/specs/ tree; cite private/specs-verified/... directly"),
                (RETIRED_PRIVATE_MAP, "the deleted PRIVATE-MAP; cite private paths directly"),
            ):
                for m in pat.finditer(text):
                    errors.append(f"{rel}: names {what}: {m.group(0)}")
            if rel not in TEST_STREAMS_ALLOWED:
                for m in RETIRED_TEST_STREAMS.finditer(text):
                    errors.append(f"{rel}: names the retired .test-streams tier (only tools/fetch-test-streams.sh may)")

        # k. ISO/IEC 14496-15 citations name the edition checked (2017; 2022 for VVC)
        if rel not in RETIRED_EXEMPT:
            for m in ISO15_UNDATED.finditer(text):
                errors.append(
                    f"{rel}: ISO/IEC 14496-15 citation must name its edition "
                    f"(14496-15:2017, or :2022 for VVC): {text[m.start():m.start() + 30]!r}"
                )

        # h. relative and helper-call fixture citations must resolve
        if rel not in RETIRED_EXEMPT:
            crate_root = rel.split("/")[0] if "/" in rel else "."
            for m in RELATIVE_FIXTURE_CITE.finditer(text):
                cite = m.group(1).rstrip(".,")
                if "{" in cite:
                    continue
                if not cite_resolves(cite, [d, crate_root], present, present_dirs):
                    errors.append(f"{rel}: relative fixture citation does not resolve: {cite}")
            if not rel.startswith(FIXTURE_CALL_EXEMPT_PREFIX):
                for m in FIXTURE_CALL.finditer(text):
                    cite = m.group(1)
                    if cite_resolves(f"fixtures/{cite}", ["."], present, present_dirs):
                        continue
                    if skip_private or cite_resolves(
                        f"private/fixtures/{cite}", ["."], set(private_files), set()
                    ):
                        continue
                    errors.append(
                        f"{rel}: test_fixtures argument resolves in neither fixtures/ nor private/fixtures/: {cite}"
                    )

        # i. source/test/doc/manifest paths named in comments and documents
        if rel not in RETIRED_EXEMPT and rel not in EXTERNAL_PATH_DOCS:
            for m in PATH_CITE.finditer(text):
                cite = m.group(1)
                first = cite.split("/")[0]
                if cite.startswith(("private/", "docs/release-notes/")):
                    continue
                if first not in CRATE_RELATIVE_ROOTS and first not in WORKSPACE_MEMBERS and first != "tools":
                    continue
                if spans and in_spans(m.start(1), spans):
                    continue
                bases = [".", crate, d]
                if first in CRATE_RELATIVE_ROOTS and first != "docs":
                    bases += sorted(WORKSPACE_MEMBERS)
                if not cite_resolves(cite, bases, present, present_dirs):
                    errors.append(f"{rel}: path does not exist: {cite}")

        # j. bare spec file names must name a real file
        if rel not in RETIRED_EXEMPT and not skip_private:
            for m in SPEC_FILE_NAME.finditer(text):
                name = m.group(1)
                if name not in known_basenames:
                    errors.append(f"{rel}: spec file name matches no file: {name}")

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

        # d. absolute github URLs
        for m in GITHUB_URL.finditer(text):
            path = m.group(1)
            if path.startswith("private/") or path.startswith("private"):
                errors.append(f"{rel}: GitHub URL links into private/: {path}")
            elif not (ROOT / path).exists():
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
