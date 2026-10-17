#!/usr/bin/env python3
"""Read-only pre-tag check of a prepared release: a PASS/FAIL table.

Usage:
    tools/finalize-release-check.py docs/release-plan-2026-10.pairs
    tools/finalize-release-check.py crate=version [crate=version ...]
    tools/finalize-release-check.py PAIRS_FILE --skip-cargo     # no cargo calls
    tools/finalize-release-check.py PAIRS_FILE --root DIR       # check another tree

Run it on the bumped tree (after `tools/prepare-release.py <pairs>` and before
the bump PR / the first tag). It writes nothing. Exit 0 only when every check
PASSes; INFO rows never fail the run.

Checks:
  manifest versions    every crate's Cargo.toml version == its pair
  lockstep             the five lockstep crates carry one identical version
  epoch-pure siblings  every versioned sibling requirement (all tables, every
                       manifest incl. demo/ fuzz/) names the epoch the sibling is
                       at: major for >=1, major.minor for 0.x (a bare "0" spans
                       epochs and fails)
  CHANGELOG            `## [ver] - date` present, `## [Unreleased]` empty
  release notes        docs/release-notes/<crate>-<ver>.md (+ v<lockstep>.md)
                       exist, non-empty, no `status: DRAFT` front matter
  README snippets      tools/readme_snippets.py agrees with the bumped versions
  Cargo.lock           contains every bumped version as a workspace member
  cargo metadata       `cargo metadata --locked --offline` succeeds
  bindings (INFO)      bindings/python + bindings/transmux-py still pin the OLD
                       published epochs; they are bumped AFTER publish
"""

from __future__ import annotations

import importlib.util
import re
import subprocess
import sys
import tomllib
from pathlib import Path

HERE = Path(__file__).resolve().parent
DEFAULT_ROOT = HERE.parent
LOCKSTEP = ("dvb-si", "dvb-t2mi", "dvb-bbframe", "dvb-conformance", "dvb-tools")
BINDINGS = ("bindings/python/Cargo.toml", "bindings/transmux-py/Cargo.toml")
DEP_TABLES = ("dependencies", "dev-dependencies", "build-dependencies")

PASS, FAIL, INFO = "PASS", "FAIL", "INFO"


def parse_pairs(args: list[str]) -> list[tuple[str, str]]:
    pairs: list[tuple[str, str]] = []
    for a in args:
        if "=" in a:
            c, v = a.split("=", 1)
            pairs.append((c, v))
        else:
            for tok in Path(a).read_text().split():
                if "=" not in tok:
                    raise SystemExit(f"{a}: bad token {tok!r}, expected <crate>=<version>")
                c, v = tok.split("=", 1)
                pairs.append((c, v))
    return pairs


def epoch(version: str) -> tuple[str, ...]:
    parts = version.split(".")
    return ("0", parts[1]) if parts[0] == "0" else (parts[0],)


def req_epoch(req: str) -> tuple[tuple[str, ...] | None, str]:
    """(epoch, problem) for a requirement string."""
    r = req.strip().lstrip("^")
    if not re.fullmatch(r"\d+(\.\d+){0,2}", r):
        return None, f"non-caret requirement {req!r}"
    parts = r.split(".")
    if parts[0] == "0":
        if len(parts) < 2:
            return None, f"requirement {req!r} spans every 0.x epoch"
        return ("0", parts[1]), ""
    return (parts[0],), ""


def _manifest_data(path: Path) -> dict:
    return tomllib.loads(path.read_text())


def workspace_manifests(root: Path) -> dict[str, tuple[Path, dict]]:
    """crate name -> (manifest path, parsed) for every `<dir>/Cargo.toml` with a
    package version."""
    out: dict[str, tuple[Path, dict]] = {}
    for m in sorted(root.glob("*/Cargo.toml")):
        d = _manifest_data(m)
        pkg = d.get("package") or {}
        if "name" in pkg and isinstance(pkg.get("version"), str):
            out[pkg["name"]] = (m, d)
    return out


def dep_entries(data: dict):
    """Yield (table, dependency crate name, spec) over every dependency table,
    including [target.*.*] ones; `package = "x"` renames are resolved."""
    tables = [("", data)] + [(f"target.{k}.", v) for k, v in (data.get("target") or {}).items()]
    for prefix, t in tables:
        for tn in DEP_TABLES:
            for name, spec in (t.get(tn) or {}).items():
                real = spec.get("package", name) if isinstance(spec, dict) else name
                yield f"{prefix}{tn}", real, spec


def short(items: list[str], n: int = 5) -> str:
    if len(items) <= n:
        return "; ".join(items)
    return "; ".join(items[:n]) + f"; ... (+{len(items) - n} more)"


def check_manifest_versions(ws, pairs):
    bad = []
    for c, v in pairs:
        if c not in ws:
            bad.append(f"{c}: no workspace manifest")
        elif ws[c][1]["package"]["version"] != v:
            bad.append(f"{c}: manifest {ws[c][1]['package']['version']} != plan {v}")
    return (FAIL, short(bad)) if bad else (PASS, f"{len(pairs)} crates at their planned version")


def check_lockstep(ws, pairs):
    d = dict(pairs)
    present = [c for c in LOCKSTEP if c in d]
    if not present:
        return INFO, "lockstep five not in this plan"
    versions = {c: ws[c][1]["package"]["version"] if c in ws else "MISSING" for c in LOCKSTEP}
    if len(set(versions.values())) != 1 or len(present) != 5:
        return FAIL, ", ".join(f"{c}={v}" for c, v in versions.items())
    return PASS, f"all five at {next(iter(versions.values()))}"


def check_epochs(root, ws, pairs):
    bad = []
    n = 0
    members = {c: d["package"]["version"] for c, (_, d) in ws.items()}
    manifests = [(c, m, d) for c, (m, d) in ws.items()]
    for extra in ("demo/Cargo.toml", "fuzz/Cargo.toml"):
        p = root / extra
        if p.is_file():
            manifests.append((extra, p, _manifest_data(p)))
    for m in sorted(root.glob("*/fuzz/Cargo.toml")) + sorted(root.glob("*/demo/Cargo.toml")):
        manifests.append((str(m.relative_to(root)), m, _manifest_data(m)))
    for label, _, data in manifests:
        for table, dep, spec in dep_entries(data):
            if dep not in members or not isinstance(spec, dict) or "version" not in spec:
                continue
            n += 1
            have, problem = req_epoch(spec["version"])
            want = epoch(members[dep])
            if problem:
                bad.append(f"{label}: {table} {dep}: {problem}")
            elif have != want:
                bad.append(f"{label}: {table} {dep} {spec['version']} but {dep} is {members[dep]}")
    return (FAIL, short(bad)) if bad else (PASS, f"{n} sibling requirements, all on the sibling's epoch")


_DATED = r"^## \[{v}\] - \d{{4}}-\d{{2}}-\d{{2}}[ \t]*$"


def check_changelogs(root, pairs):
    bad = []
    for c, v in pairs:
        p = root / c / "CHANGELOG.md"
        if not p.is_file():
            bad.append(f"{c}: no CHANGELOG.md")
            continue
        text = p.read_text()
        if not re.search(_DATED.format(v=re.escape(v)), text, re.M):
            bad.append(f"{c}: no `## [{v}] - <date>` heading")
        m = re.search(r"^## \[Unreleased\][ \t]*$", text, re.M)
        if m:
            nxt = re.search(r"^## ", text[m.end() :], re.M)
            body = text[m.end() : m.end() + nxt.start()] if nxt else text[m.end() :]
            if body.strip():
                bad.append(f"{c}: leftover [Unreleased] entries")
    return (FAIL, short(bad)) if bad else (PASS, f"{len(pairs)} CHANGELOGs cut, [Unreleased] empty")


def _is_draft(text: str) -> bool:
    m = re.match(r"\A---[ \t]*\n(.*?)\n---[ \t]*(?:\n|\Z)", text, re.S)
    return bool(m and re.search(r"^status:\s*DRAFT\s*$", m.group(1), re.M))


def check_notes(root, pairs):
    names = [f"{c}-{v}.md" for c, v in pairs]
    d = dict(pairs)
    lock = {d[c] for c in LOCKSTEP if c in d}
    if len(lock) == 1:
        names.append(f"v{next(iter(lock))}.md")
    bad = []
    for n in names:
        p = root / "docs" / "release-notes" / n
        if not p.is_file():
            bad.append(f"{n}: missing")
            continue
        text = p.read_text()
        if not text.strip():
            bad.append(f"{n}: empty")
        elif _is_draft(text):
            bad.append(f"{n}: still `status: DRAFT`")
    return (FAIL, short(bad)) if bad else (PASS, f"{len(names)} notes present, none DRAFT")


def _load(name: str, file: str):
    spec = importlib.util.spec_from_file_location(name, HERE / file)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def check_readmes(root, pairs):
    rs = _load("readme_snippets", "readme_snippets.py")
    bad = rs.check(root, rs.plan_versions(pairs, root))
    return (FAIL, short(bad)) if bad else (PASS, "every README dependency snippet matches")


def check_lock(root, pairs):
    p = root / "Cargo.lock"
    if not p.is_file():
        return FAIL, "no Cargo.lock"
    have = {(q["name"], q["version"]) for q in tomllib.loads(p.read_text()).get("package", []) if "source" not in q}
    bad = [f"{c} {v}" for c, v in pairs if (c, v) not in have]
    return (FAIL, "not in Cargo.lock as a member: " + short(bad)) if bad else (PASS, f"{len(pairs)} bumped versions are in Cargo.lock")


def check_metadata_locked(root):
    try:
        r = subprocess.run(
            ["cargo", "metadata", "--locked", "--offline", "--format-version", "1"],
            cwd=root,
            capture_output=True,
            text=True,
        )
    except FileNotFoundError:
        return FAIL, "cargo not found"
    if r.returncode != 0:
        return FAIL, short([l for l in r.stderr.strip().splitlines() if l][:3])
    return PASS, "cargo metadata --locked --offline OK"


def check_bindings(root, pairs):
    new = dict(pairs)
    notes = []
    for rel in BINDINGS:
        p = root / rel
        if not p.is_file():
            notes.append(f"{rel}: absent")
            continue
        for table, dep, spec in dep_entries(_manifest_data(p)):
            if dep not in new:
                continue
            req = spec.get("version") if isinstance(spec, dict) else spec
            have, problem = req_epoch(req or "")
            state = "ALREADY on new epoch" if have == epoch(new[dep]) else "pins OLD (expected pre-publish)"
            notes.append(f"{Path(rel).parent.name}: {dep} {req} vs planned {new[dep]}: {state}")
    return INFO, short(notes, 8) or "no bindings depend on a bumped crate"


def run_checks(root: Path, pairs: list[tuple[str, str]], skip_cargo: bool = False) -> list[tuple[str, str, str]]:
    ws = workspace_manifests(root)
    rows = [
        ("manifest versions", *check_manifest_versions(ws, pairs)),
        ("lockstep five identical", *check_lockstep(ws, pairs)),
        ("epoch-pure siblings", *check_epochs(root, ws, pairs)),
        ("CHANGELOG cut", *check_changelogs(root, pairs)),
        ("release notes final", *check_notes(root, pairs)),
        ("README snippets", *check_readmes(root, pairs)),
        ("Cargo.lock versions", *check_lock(root, pairs)),
    ]
    if skip_cargo:
        rows.append(("cargo metadata --locked", INFO, "skipped (--skip-cargo)"))
    else:
        rows.append(("cargo metadata --locked", *check_metadata_locked(root)))
    rows.append(("bindings pin old (report)", *check_bindings(root, pairs)))
    return rows


def render(rows: list[tuple[str, str, str]]) -> str:
    w = max(len(r[0]) for r in rows)
    lines = [f"{'CHECK'.ljust(w)}  RESULT  DETAIL", f"{'-' * w}  ------  ------"]
    lines += [f"{n.ljust(w)}  {s:<6}  {d}" for n, s, d in rows]
    return "\n".join(lines)


def main(argv: list[str]) -> int:
    args = list(argv[1:])
    skip = "--skip-cargo" in args
    args = [a for a in args if a != "--skip-cargo"]
    root = DEFAULT_ROOT
    if "--root" in args:
        i = args.index("--root")
        root = Path(args[i + 1]).resolve()
        del args[i : i + 2]
    if not args:
        print(__doc__, file=sys.stderr)
        return 2
    pairs = parse_pairs(args)
    rows = run_checks(root, pairs, skip)
    print(render(rows))
    failed = [n for n, s, _ in rows if s == FAIL]
    print(f"\n{'FAILED: ' + ', '.join(failed) if failed else 'ALL CHECKS PASS'} ({len(pairs)} crates, root {root})")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
