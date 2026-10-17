#!/usr/bin/env python3
"""Prepare a release: bump versions, cut CHANGELOGs, scaffold release notes.

Usage:
    tools/prepare-release.py <crate>=<version> [<crate>=<version> ...]
    tools/prepare-release.py --check <crate>=<version> [...]     # dry run
    tools/prepare-release.py --verify <crate>=<version> [...]    # post-publish
    tools/prepare-release.py --tag-order <crate>=<version> [...]  # tags, push order
    tools/prepare-release.py --tag-needs <crate>=<version> [...]  # same, as the plan's section-6 markdown
    tools/prepare-release.py --check-readmes [<crate>=<version> ...]

Apply/--check options: --no-lock, --no-finalize-notes, --extra-note PATH (repeatable).
                                    # exit 1 if a README `crate = "x.y"` snippet
                                    # disagrees with the current/planned version

Does the mechanical, error-prone half of `docs/RELEASE-DOCS.md` so it is not
retyped by hand each time:

1. **Version bump** — sets `version = "X.Y.Z"` in each crate's `Cargo.toml`.
2. **Sibling dep refs** — updates every OTHER workspace crate that depends on a
   bumped crate to the new caret version, so no published bucket ends up
   spanning two epochs (the #858 rule).
2b. **README snippets** — rewrites every README `crate = "x.y"` /
   `crate = { version = "x.y", .. }` dependency snippet for a workspace crate
   to the bump plan's version (`tools/readme_snippets.py`; the dry run lists
   the rewrites, `--check-readmes` fails on any disagreement).
3. **CHANGELOG cut** — `## [Unreleased]` becomes `## [X.Y.Z] - YYYY-MM-DD` and
   a fresh empty `## [Unreleased]` goes back on top.
4. **Release note** — scaffolds `docs/release-notes/<crate>-<version>.md` from
   the CHANGELOG section just cut, if one does not already exist.
4b. **Finalise notes** — strips the `status: DRAFT` front-matter block from the
   note of every bumped pair, from `v<lockstep>.md` when the lockstep five are
   bumped, and from any `--extra-note PATH`. Fails loudly (before anything is
   edited) on a note with no front matter, unexpected front matter, or a
   placeholder marker (a line starting `TODO`/`TBD`/`DRAFT`, or `[TODO]`,
   `<TBD>`...). Skip with `--no-finalize-notes` (old scaffold-and-edit flow).
4c. **Crate index** — runs `tools/gen-crate-index.py` (docs/CRATES.md and the
   README crate table derive from the manifests) and then its `--check`.
4d. **Cargo.lock** — runs `cargo metadata --format-version 1 --offline`, which
   rewrites ONLY the workspace members' own `version` lines in the committed
   lock (every third-party pin, which is MSRV-compatible by policy, is kept),
   then verifies the lock diff touches nothing else (restoring the old lock if
   it does) and that `cargo metadata --locked --offline` passes. The committed
   `demo/Cargo.lock` (its own workspace) gets the same guarded refresh. Skip
   with `--no-lock`.
5. **Tag order** — prints the dependency-correct tag order (normal, build AND
   versioned dev-dependency edges; the lockstep five collapse into one
   `v<ver>` slot), so a crate is never tagged before something it depends on
   is live. `--tag-order <pairs...>` prints just that, one tag per line.

It does NOT tag, push, or publish. Those stay manual and need explicit
sign-off; `tools/push-release-tags.sh` pushes tags one at a time afterwards.

`--verify` checks each crate is live at the expected version via
`tools/crates-io.py`, which is the only supported way to ask crates.io (it
sends the User-Agent crates.io requires, and never reports a failed request as
"not published").
"""

from __future__ import annotations

import datetime as _dt
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LOCKSTEP = ("dvb-si", "dvb-t2mi", "dvb-bbframe", "dvb-conformance", "dvb-tools")
TODAY = _dt.date.today().isoformat()

VERSION_RE = re.compile(r'^(version\s*=\s*)"([^"]+)"', re.M)


def crate_manifest(crate: str) -> Path:
    p = ROOT / crate / "Cargo.toml"
    if not p.is_file():
        raise SystemExit(f"no manifest for crate {crate!r} at {p}")
    return p


def current_version(crate: str) -> str:
    m = VERSION_RE.search(crate_manifest(crate).read_text())
    if not m:
        raise SystemExit(f"{crate}: no `version = \"...\"` in Cargo.toml")
    return m.group(2)


def bump_version(crate: str, new: str, apply: bool) -> str:
    path = crate_manifest(crate)
    text = path.read_text()
    old = current_version(crate)
    if old == new:
        return f"  {crate}: already at {new}"
    text2 = VERSION_RE.sub(lambda m: f'{m.group(1)}"{new}"', text, count=1)
    if apply:
        path.write_text(text2)
    return f"  {crate}: {old} -> {new}"


def caret(version: str) -> str:
    """The caret-range string a sibling should depend on: always major.minor.

    For 0.x that is the epoch itself. For >=1.0, writing only the major (e.g.
    "9") would let a dependent that uses an API added in 9.4 publish with a
    requirement 9.3 satisfies — a broken minimal version. major.minor keeps the
    same caret epoch while pinning the floor to the release that has the API."""
    parts = version.split(".")
    return f"{parts[0]}.{parts[1]}"


def manifests() -> list[Path]:
    """Every workspace-member manifest plus the fuzz/ and demo/ manifests that
    sit beside or under a member (they carry dependency requirements too).

    Deliberately NOT covered, both harmless: `spikes/trunk-bench` (no sibling
    dependency at all) and `tools/differential-sweep` (every sibling dependency
    is `path`-only with NO `version`, so there is no requirement to bump and
    nothing to drift; `tests/test_release_tooling.py` pins both facts, so the day
    one gains a versioned sibling requirement the test fails and this list must
    grow)."""
    found: set[Path] = set()
    for pat in ("*/Cargo.toml", "fuzz/Cargo.toml", "demo/Cargo.toml", "*/fuzz/Cargo.toml", "*/demo/Cargo.toml"):
        found.update(ROOT.glob(pat))
    return sorted(found)


def dep_regex(crate: str) -> re.Pattern[str]:
    return re.compile(
        rf'(^[ \t]*{re.escape(crate)}\s*=\s*\{{[^}}]*?version\s*=\s*)"([^"]+)"',
        re.M,
    )


def update_sibling_refs(crate: str, new: str, apply: bool) -> list[str]:
    """Point EVERY dependency entry on `crate` (in every table of every
    manifest -- [dependencies], [dev-dependencies], [build-dependencies],
    [target.*.*]) at `new`.

    The 2026-10-13 wave left stale refs (media-doctor's `transmux`, multimux's
    dev-dependency `webrtc-runtime`) because this used `search()` on the first
    match only and skipped the whole file when THAT match was already current,
    so a second entry in a later table was never looked at."""
    out: list[str] = []
    want = caret(new)
    dep_re = dep_regex(crate)
    for manifest in manifests():
        if manifest.parent.name == crate and manifest.parent.parent == ROOT:
            continue
        text = manifest.read_text()
        stale = [m for m in dep_re.finditer(text) if m.group(2) != want]
        if not stale:
            continue
        if apply:
            manifest.write_text(dep_re.sub(lambda mm: f'{mm.group(1)}"{want}"', text))
        label = manifest.relative_to(ROOT).parent
        for m in stale:
            out.append(f"  {label}: dep {crate} {m.group(2)} -> {want}")
    return out


def depends_on_any(crate: str, bumped: list[str]) -> list[str]:
    """Which of the `bumped` crates does `crate`'s manifest depend on?"""
    text = crate_manifest(crate).read_text()
    return [b for b in bumped if b != crate and dep_regex(b).search(text)]


def verify_resolution(root: Path | None = None) -> int:
    """After the edits, make sure the workspace still resolves, offline and
    against the committed lock. A stale sibling requirement shows up here."""
    r = subprocess.run(
        ["cargo", "metadata", "--locked", "--offline", "--format-version", "1"],
        cwd=root or ROOT,
        capture_output=True,
        text=True,
    )
    if r.returncode != 0:
        print(
            "\nERROR: `cargo metadata --locked --offline` failed after the edits -- "
            "a dependency requirement or Cargo.lock is inconsistent:\n" + r.stderr,
            file=sys.stderr,
        )
        return 1
    print("\nResolution check: cargo metadata --locked --offline OK")
    return 0


def cut_changelog(crate: str, new: str, apply: bool, default_note: str = "") -> tuple[str, str]:
    """Turn `## [Unreleased]` into a dated version heading. Returns
    (status, the section body that was cut) for the release note."""
    path = ROOT / crate / "CHANGELOG.md"
    if not path.is_file():
        return (f"  {crate}: NO CHANGELOG", "")
    text = path.read_text()
    if f"## [{new}]" in text:
        # Already cut — or, for a brand-new crate, the version section was
        # written directly with no `[Unreleased]` phase. Either way the section
        # body is what the release note should be built from, so return it
        # rather than an empty string (which would silently skip the note).
        m_existing = re.search(rf"^## \[{re.escape(new)}\].*$", text, re.M)
        body = ""
        if m_existing:
            start = m_existing.end()
            nxt = re.search(r"^## ", text[start:], re.M)
            body = (text[start : start + nxt.start()] if nxt else text[start:]).strip()
        return (f"  {crate}: CHANGELOG already has [{new}]", body)
    m = re.search(r"^## \[Unreleased\]\s*$", text, re.M)
    if not m:
        return (f"  {crate}: no `## [Unreleased]` heading", "")

    start = m.end()
    nxt = re.search(r"^## ", text[start:], re.M)
    body = text[start : start + nxt.start()] if nxt else text[start:]
    empty = not body.strip()
    if empty:
        # An empty [Unreleased] on a crate that is being released (a dependency
        # or epoch bump) still needs a dated section and a release note, or the
        # release lane's "release note exists" gate fails (2026-10-13 wave).
        note = default_note or "Dependency/epoch bump only; no functional change."
        body = f"\n\n### Changed\n- {note}\n\n"
        replacement = f"## [Unreleased]\n\n## [{new}] - {TODAY}{body}"
        if apply:
            path.write_text(text[: m.start()] + replacement + text[start:].lstrip("\n"))
        return (f"  {crate}: [Unreleased] was EMPTY -- cut [{new}] with a default `### Changed` line", f"### Changed\n- {note}")

    replacement = f"## [Unreleased]\n\n## [{new}] - {TODAY}"
    if apply:
        path.write_text(text[: m.start()] + replacement + text[start:])
    return (f"  {crate}: CHANGELOG cut to [{new}] - {TODAY}", body.strip())


def scaffold_release_note(crate: str, new: str, body: str, apply: bool) -> str:
    path = ROOT / "docs" / "release-notes" / f"{crate}-{new}.md"
    if path.exists():
        return f"  {crate}: release note already exists ({path.name})"
    if not body:
        return f"  {crate}: no CHANGELOG body — release note NOT scaffolded"
    content = f"""# {crate} {new}

_Released {TODAY}._

{body}

---

Published from tag `{crate}-v{new}`.
"""
    if apply:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)
    return f"  {crate}: release note scaffolded ({path.name}) — REVIEW AND EDIT IT"


def _publish_deps(manifest: Path) -> set[str]:
    """Names in [dependencies] / [build-dependencies] (incl. target-specific
    tables), plus VERSIONED dev-dependencies: `cargo publish` resolves a
    dev-dependency that carries a `version` against the index (mpeg-ts's dev-dep
    on mp4-emsg gates its publish), while a path-only dev-dependency is stripped
    and cannot gate anything -- counting those created cycles that used to
    collapse the order to alphabetical."""
    import tomllib

    data = tomllib.loads(manifest.read_text())
    names: set[str] = set()

    def take(table: dict) -> None:
        for k in ("dependencies", "build-dependencies"):
            for name, spec in (table.get(k) or {}).items():
                names.add(spec.get("package", name) if isinstance(spec, dict) else name)
        for name, spec in (table.get("dev-dependencies") or {}).items():
            if isinstance(spec, dict) and "version" in spec:
                names.add(spec.get("package", name))

    take(data)
    for tgt in (data.get("target") or {}).values():
        take(tgt)
    return names


def dependency_order(crates: list[str]) -> list[str]:
    """Order crates so a dependency is tagged before its dependents."""
    deps: dict[str, set[str]] = {}
    for c in crates:
        deps[c] = (_publish_deps(crate_manifest(c)) & set(crates)) - {c}
    ordered: list[str] = []
    remaining = dict(deps)
    while remaining:
        ready = sorted(c for c, d in remaining.items() if not (d - set(ordered)))
        if not ready:
            raise SystemExit(f"dependency cycle among: {sorted(remaining)}")
        ordered.extend(ready)
        for c in ready:
            remaining.pop(c)
    return ordered


def lockstep_version(pairs: list[tuple[str, str]]) -> str | None:
    """The lockstep version if any of the five is in `pairs`; all five must be
    present and agree, else SystemExit."""
    d = dict(pairs)
    present = [c for c in LOCKSTEP if c in d]
    if not present:
        return None
    missing = [c for c in LOCKSTEP if c not in d]
    versions = {d[c] for c in present}
    if missing or len(versions) != 1:
        raise SystemExit(f"lockstep five must be bumped together to ONE version: missing={missing} versions={sorted(versions)}")
    return versions.pop()


def tag_needs(pairs: list[tuple[str, str]]) -> dict[str, list[str]]:
    """tag -> the direct dependencies (crate names; `v<ver>` for the lockstep
    slot, listed first) that must be live before that tag is pushed."""
    ls = lockstep_version(pairs)
    versions = dict(pairs)
    node_of = {c: (f"v{ls}" if c in LOCKSTEP else f"{c}-v{versions[c]}") for c in versions}
    needs: dict[str, set[str]] = {}
    for c in versions:
        n = node_of[c]
        for d in (_publish_deps(crate_manifest(c)) & set(versions)) - {c}:
            if node_of[d] != n:
                needs.setdefault(n, set()).add(node_of[d] if d in LOCKSTEP else d)
        needs.setdefault(n, set())
    return {n: sorted(v, key=lambda x: (not x.startswith("v"), x)) for n, v in needs.items()}


def tag_needs_markdown(pairs: list[tuple[str, str]]) -> str:
    """The numbered list of docs/release-plan-2026-10.md section 6 (generated:
    `--tag-order` is the single source of truth; a test pins the doc to it)."""
    needs = tag_needs(pairs)
    lines: list[str] = []
    for i, t in enumerate(tag_order(pairs), 1):
        n = needs[t]
        what = ", ".join(f"`{x}`" for x in n) if n else "(nothing in this wave)"
        lines.append(f"{i}. `{t}`  ")
        extra = ""
        if t.startswith("v") and t[1].isdigit():
            extra = "  \n   five crates in one lane, publishes bbframe -> si -> conformance -> t2mi -> tools; **push by hand** (`push-release-tags.sh` refuses `v*` in explicit-tag mode; `--plan` stops here, exit 2)"
        lines.append(f"   needs live first: {what}{extra}")
    return "\n".join(lines)


def tag_order(pairs: list[tuple[str, str]]) -> list[str]:
    """Push order as tags: dependencies first, the lockstep five collapsed into
    one `v<ver>` slot (alphabetical within a ready batch, as the plan lists)."""
    ls = lockstep_version(pairs)
    versions = dict(pairs)
    node_of = {c: (f"v{ls}" if c in LOCKSTEP else f"{c}-v{versions[c]}") for c in versions}
    deps: dict[str, set[str]] = {}
    for c in versions:
        crate_deps = {node_of[d] for d in (_publish_deps(crate_manifest(c)) & set(versions)) if d != c}
        deps.setdefault(node_of[c], set()).update(crate_deps)
    for n in deps:
        deps[n].discard(n)
    ordered: list[str] = []
    remaining = dict(deps)
    while remaining:
        ready = sorted(
            (n for n, d in remaining.items() if not (d - set(ordered))),
            key=lambda n: (not n.startswith("broadcast-common-v"), n),  # the root crate leads
        )
        if not ready:
            raise SystemExit(f"dependency cycle among: {sorted(remaining)}")
        ordered.extend(ready)
        for n in ready:
            remaining.pop(n)
    return ordered


_PLACEHOLDER_RE = re.compile(
    r"^[ \t]*(?:[-*>][ \t]+)?(?:\*\*)?(?:DRAFT|TODO|TBD)\b(?!\()"  # a line that STARTS with the marker
    r"|\[(?:DRAFT|TODO|TBD)\]|<(?:DRAFT|TODO|TBD)>",
    re.M,
)


def note_path_for(crate_or_tag: str, version: str | None = None) -> Path:
    base = ROOT / "docs" / "release-notes"
    return base / (f"{crate_or_tag}.md" if version is None else f"{crate_or_tag}-{version}.md")


def notes_to_finalize(pairs: list[tuple[str, str]], extra: list[str]) -> list[Path]:
    paths = [note_path_for(c, v) for c, v in pairs]
    ls = lockstep_version(pairs)
    if ls:
        paths.append(note_path_for(f"v{ls}"))
    paths.extend((ROOT / e if not Path(e).is_absolute() else Path(e)) for e in extra)
    return paths


def strip_draft_front_matter(text: str) -> tuple[str | None, str]:
    """(new_text, problem). `problem` is "" when the note is finalisable. Only a
    leading `---` block holding exactly `status: DRAFT` is removed; the body is
    kept byte for byte."""
    lines = text.split("\n")
    if not lines or lines[0].strip() != "---":
        return None, "has no front matter (expected a leading `---` block with `status: DRAFT`)"
    try:
        end = next(i for i in range(1, len(lines)) if lines[i].strip() == "---")
    except StopIteration:
        return None, "front matter is not closed with `---`"
    block = [l.strip() for l in lines[1:end] if l.strip()]
    if block != ["status: DRAFT"]:
        return None, f"unexpected front matter {block!r} (only `status: DRAFT` may be stripped)"
    body = "\n".join(lines[end + 1 :])
    m = _PLACEHOLDER_RE.search(body)
    if m:
        line = body.count("\n", 0, m.start()) + 1
        return None, f"still contains placeholder text {m.group(0).strip()!r} (body line {line})"
    if not body.strip():
        return None, "has no body"
    return body, ""


def finalize_notes(paths: list[Path], apply: bool) -> tuple[list[str], list[str]]:
    """Strip the DRAFT front matter from every note. Returns (log, problems);
    nothing is written unless `apply` AND there are no problems at all."""
    log: list[str] = []
    problems: list[str] = []
    todo: list[tuple[Path, str]] = []
    for p in paths:
        rel = p.relative_to(ROOT) if p.is_relative_to(ROOT) else p
        if not p.is_file():
            problems.append(f"  {rel}: MISSING release note")
            continue
        new, problem = strip_draft_front_matter(p.read_text())
        if problem:
            problems.append(f"  {rel}: {problem}")
        else:
            todo.append((p, new))
            log.append(f"  {rel}: finalised (status: DRAFT front matter stripped)")
    if apply and not problems:
        for p, new in todo:
            p.write_text(new)
    return log, problems


def run_gen_crate_index(apply: bool) -> int:
    script = ROOT / "tools" / "gen-crate-index.py"
    if not apply:
        print("  would run: python3 tools/gen-crate-index.py  (regenerates docs/CRATES.md + README crate table), then --check")
        return 0
    for extra in ([], ["--check"]):
        r = subprocess.run([sys.executable, str(script), *extra], cwd=ROOT, capture_output=True, text=True)
        sys.stdout.write("".join(f"  {l}\n" for l in (r.stdout + r.stderr).splitlines()))
        if r.returncode != 0:
            print(f"ERROR: gen-crate-index.py {' '.join(extra)} failed", file=sys.stderr)
            return 1
    return 0


def _lock_packages(text: str) -> list[dict]:
    import tomllib

    return tomllib.loads(text).get("package", [])


def lock_violations(old: str, new: str, allow_member_set_change: bool = False) -> list[str]:
    """Differences between two Cargo.lock texts OTHER than the workspace
    members' own `version` lines (a member = a package with no `source`).
    Third-party entries must be byte-identical as parsed; member entries must
    match once `version` is dropped and member-to-member dependency strings are
    reduced to the bare name. `allow_member_set_change` tolerates a path package
    appearing/disappearing (demo/'s lock was found missing `test-fixtures`)."""
    po, pn = _lock_packages(old), _lock_packages(new)
    members = {p["name"] for p in pn if "source" not in p} | {p["name"] for p in po if "source" not in p}

    def norm(p: dict) -> str:
        q = dict(p)
        if "source" not in q:
            q.pop("version", None)
            q["dependencies"] = sorted(
                d.split(" ")[0] if d.split(" ")[0] in members and len(d.split(" ")) == 2 else d
                for d in q.get("dependencies", [])
            )
        return json.dumps(q, sort_keys=True)

    def index(pkgs: list[dict]) -> dict[str, str]:
        return {
            (f"{p['name']}@member" if "source" not in p else f"{p['name']}@{p['version']}@{p['source']}"): norm(p)
            for p in pkgs
        }

    io, inn = index(po), index(pn)
    bad: list[str] = []
    for k in sorted(set(io) | set(inn)):
        if allow_member_set_change and k.endswith("@member") and (k not in io or k not in inn):
            continue
        if k not in inn:
            bad.append(f"  removed from Cargo.lock: {k}")
        elif k not in io:
            bad.append(f"  added to Cargo.lock: {k}")
        elif io[k] != inn[k]:
            bad.append(f"  changed in Cargo.lock beyond a member version: {k}")
    return bad


def refresh_lock(apply: bool) -> int:
    """Rewrite only the workspace members' versions in Cargo.lock.

    `cargo update --workspace` is NOT used: it re-resolves and can float
    third-party pins past the MSRV-compatible ones the committed lock holds.
    `cargo metadata --offline` resolves against the lock and only edits what the
    manifests force -- here, each bumped member's own version -- and `--offline`
    makes it fail rather than reach for the network or a newer index."""
    cmd = ["cargo", "metadata", "--format-version", "1", "--offline"]
    if not apply:
        print(f"  would run: {' '.join(cmd)} > /dev/null, assert the lock diff is member-version lines only,")
        print("  then `cargo metadata --locked --offline` (skip with --no-lock)")
        return 0
    lock = ROOT / "Cargo.lock"
    old = lock.read_text()
    r = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
    if r.returncode != 0:
        print(f"ERROR: `{' '.join(cmd)}` failed:\n{r.stderr}", file=sys.stderr)
        lock.write_text(old)
        return 1
    new = lock.read_text()
    bad = lock_violations(old, new)
    if bad:
        lock.write_text(old)
        print("ERROR: the lock refresh changed more than workspace-member versions; Cargo.lock RESTORED:", file=sys.stderr)
        print("\n".join(bad), file=sys.stderr)
        return 1
    n = sum(1 for a, b in zip(old.splitlines(), new.splitlines()) if a != b) if old != new else 0
    print(f"  Cargo.lock refreshed: {n} line(s) changed, all workspace-member versions")
    return 0


def refresh_demo_lock(apply: bool) -> int:
    """demo/ is its own (excluded) workspace with a COMMITTED Cargo.lock that
    records the path members' versions; a bump leaves it stale. pages.yml does not
    build with --locked, so a stale lock self-heals there (cargo rewrites it) and
    never fails a lane, but a committed lock that lags the manifests is exactly
    the rot this tool exists to remove. Same guard as the root lock: only
    member versions (or a path member appearing) may change, else restore.

    A failure to resolve offline (a wasm dependency missing from the local
    cargo cache) is a WARNING, not an error: it cannot affect any release."""
    lock = ROOT / "demo" / "Cargo.lock"
    manifest = ROOT / "demo" / "Cargo.toml"
    if not (lock.is_file() and manifest.is_file()):
        print("  demo/Cargo.lock: absent, nothing to refresh")
        return 0
    cmd = ["cargo", "metadata", "--format-version", "1", "--offline", "--manifest-path", "demo/Cargo.toml"]
    if not apply:
        print(f"  would run: {' '.join(cmd)} > /dev/null (refreshes demo/Cargo.lock member versions only)")
        return 0
    old = lock.read_text()
    r = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
    if r.returncode != 0:
        lock.write_text(old)
        print(f"  WARNING: demo/Cargo.lock NOT refreshed (`{' '.join(cmd)}` failed offline; pages.yml is not --locked so this is harmless):")
        print("  " + " | ".join(l for l in r.stderr.strip().splitlines()[-2:]))
        return 0
    new = lock.read_text()
    bad = lock_violations(old, new, allow_member_set_change=True)
    if bad:
        lock.write_text(old)
        print("ERROR: the demo lock refresh changed more than workspace-member versions; demo/Cargo.lock RESTORED:", file=sys.stderr)
        print("\n".join(bad), file=sys.stderr)
        return 1
    n = sum(1 for a, b in zip(old.splitlines(), new.splitlines()) if a != b) if old != new else 0
    print(f"  demo/Cargo.lock refreshed: {n} line(s) changed, all workspace-member entries")
    return 0


def verify(pairs: list[tuple[str, str]]) -> int:
    args = [f"{c}={v}" for c, v in pairs]
    return subprocess.call([sys.executable, str(ROOT / "tools" / "crates-io.py"), "check", *args])


def restamp_dates(pairs: list[tuple[str, str]], apply: bool) -> int:
    """Set every CHANGELOG heading and release-note `_Released_` line for this
    wave to today.

    A wave staged over several days ends up with headings dated whenever each
    crate happened to be prepared -- this wave carried a mix of 2026-08-12 and
    2026-08-13 while the code kept changing. Every one of those dates is a claim
    about when the crate was PUBLISHED, and all of them were wrong, because
    nothing had been published at all.

    The dates are therefore not set when the changelog is cut; they are stamped
    at sign-off, in one command, so they agree with each other and with reality.
    Run this immediately before pushing the tags.

    NOTE the character class: `[ \\t]*$`, never `\\s*$`. Under `re.MULTILINE`,
    `\\s` matches newlines, so `\\s*$` swallows the blank line that follows the
    heading and silently reflows the document -- observed on the first real run
    of this function, which glued "### Changed" onto the version heading in five
    files at once.
    """
    rc = 0
    for crate, version in pairs:
        ch = ROOT / crate / "CHANGELOG.md"
        if ch.is_file():
            text = ch.read_text()
            pat = re.compile(rf"^## \[{re.escape(version)}\] - \d{{4}}-\d{{2}}-\d{{2}}[ \t]*$", re.M)
            m = pat.search(text)
            if not m:
                print(f"  {crate}: no dated `## [{version}]` heading to restamp")
                rc = 1
            else:
                was = m.group(0).strip()
                new = f"## [{version}] - {TODAY}"
                if was == new:
                    print(f"  {crate}: CHANGELOG already {TODAY}")
                else:
                    if apply:
                        ch.write_text(pat.sub(new, text, count=1))
                    print(f"  {crate}: CHANGELOG {was} -> {new}")
        else:
            print(f"  {crate}: NO CHANGELOG")
            rc = 1

        note = ROOT / "docs" / "release-notes" / f"{crate}-{version}.md"
        if note.is_file():
            text = note.read_text()
            pat = re.compile(r"^_Released \d{4}-\d{2}-\d{2}\._[ \t]*$", re.M)
            m = pat.search(text)
            if not m:
                print(f"  {crate}: release note has no `_Released <date>._` line")
                rc = 1
            else:
                was = m.group(0).strip()
                new = f"_Released {TODAY}._"
                if was == new:
                    print(f"  {crate}: release note already {TODAY}")
                else:
                    if apply:
                        note.write_text(pat.sub(new, text, count=1))
                    print(f"  {crate}: note {was} -> {new}")
        else:
            print(f"  {crate}: NO release note docs/release-notes/{crate}-{version}.md")
            rc = 1
    return rc


def _readme_snippets():
    """Import the sibling tools/readme_snippets.py module."""
    import importlib.util

    spec = importlib.util.spec_from_file_location("readme_snippets", Path(__file__).resolve().parent / "readme_snippets.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def check_readmes(pairs: list[tuple[str, str]]) -> int:
    """Fail when any README dependency snippet disagrees with the crate's
    current manifest version (overlaid by the planned `pairs`, if given)."""
    snippets = _readme_snippets()
    bad = snippets.check(ROOT, snippets.plan_versions(pairs, ROOT))
    for line in bad:
        print(line)
    if bad:
        print(f"\n{len(bad)} stale README dependency snippet(s); the apply path rewrites them.", file=sys.stderr)
        return 1
    print("README dependency snippets OK")
    return 0


def _split_flags(args: list[str]) -> tuple[list[str], dict]:
    opts = {"lock": True, "finalize": True, "extra": []}
    rest: list[str] = []
    it = iter(args)
    for a in it:
        if a == "--no-lock":
            opts["lock"] = False
        elif a == "--no-finalize-notes":
            opts["finalize"] = False
        elif a == "--extra-note":
            try:
                opts["extra"].append(next(it))
            except StopIteration:
                raise SystemExit("--extra-note needs a PATH") from None
        elif a.startswith("--extra-note="):
            opts["extra"].append(a.split("=", 1)[1])
        else:
            rest.append(a)
    return rest, opts


def main(argv: list[str]) -> int:
    args, opts = _split_flags(argv[1:])
    if not args:
        print(__doc__, file=sys.stderr)
        return 2

    mode = "apply"
    if args[0] == "--check-readmes":
        mode, args = "check-readmes", args[1:]
    elif args[0] == "--check":
        mode, args = "check", args[1:]
    elif args[0] == "--verify":
        mode, args = "verify", args[1:]
    elif args[0] == "--restamp":
        mode, args = "restamp", args[1:]
    elif args[0] == "--restamp-check":
        mode, args = "restamp-check", args[1:]
    elif args[0] == "--tag-order":
        mode, args = "tag-order", args[1:]
    elif args[0] == "--tag-needs":
        mode, args = "tag-needs", args[1:]

    pairs: list[tuple[str, str]] = []
    for a in args:
        if "=" not in a:
            raise SystemExit(f"bad argument {a!r}, expected <crate>=<version>")
        c, v = a.split("=", 1)
        pairs.append((c, v))

    if mode == "verify":
        return verify(pairs)

    if mode == "tag-order":
        print("\n".join(tag_order(pairs)))
        return 0

    if mode == "tag-needs":
        print(tag_needs_markdown(pairs))
        return 0

    if mode == "check-readmes":
        return check_readmes(pairs)

    if mode in ("restamp", "restamp-check"):
        doing = mode == "restamp"
        print(f"{'RESTAMPING' if doing else 'DRY RUN --'} release dates to {TODAY}\n")
        return restamp_dates(pairs, doing)

    apply = mode == "apply"
    print(f"{'APPLYING' if apply else 'DRY RUN —'} release prep for {len(pairs)} crate(s)\n")

    # Preflight: refuse to start editing if a note cannot be finalised, so a
    # failure never leaves a half-applied tree behind.
    if opts["finalize"]:
        _, problems = finalize_notes(notes_to_finalize(pairs, opts["extra"]), False)
        if problems:
            print("Release notes that cannot be finalised (nothing was edited):", file=sys.stderr)
            print("\n".join(problems), file=sys.stderr)
            print("Fix the notes, or pass --no-finalize-notes to scaffold-and-edit by hand.", file=sys.stderr)
            return 1

    print("Version bumps:")
    for c, v in pairs:
        print(bump_version(c, v, apply))

    print("\nSibling dependency refs:")
    any_ref = False
    for c, v in pairs:
        for line in update_sibling_refs(c, v, apply):
            print(line)
            any_ref = True
    if not any_ref:
        print("  (none needed)")

    print("\nREADME dependency snippets:")
    snippets = _readme_snippets()
    planned = snippets.plan_versions(pairs, ROOT)
    for line in snippets.rewrite(ROOT, planned, apply) or ["  (none needed)"]:
        print(line)

    print("\nCHANGELOGs:")
    bodies: dict[str, str] = {}
    bumped = [c for c, _ in pairs]
    for c, v in pairs:
        via = depends_on_any(c, bumped)
        note = (
            f"Dependency/epoch bump only (follows {', '.join(via)}); no functional change."
            if via
            else ""
        )
        status, body = cut_changelog(c, v, apply, note)
        print(status)
        bodies[c] = body

    print("\nRelease notes:")
    for c, v in pairs:
        print(scaffold_release_note(c, v, bodies.get(c, ""), apply))

    if opts["finalize"]:
        print("\nFinalise release notes (strip `status: DRAFT` front matter):")
        log, problems = finalize_notes(notes_to_finalize(pairs, opts["extra"]), apply)
        print("\n".join(log + problems))
        if problems:
            print("ERROR: a scaffolded note still has no front matter; edit it, then re-run with --no-finalize-notes", file=sys.stderr)
            return 1

    print("\nCrate index (docs/CRATES.md, README crate table):")
    if run_gen_crate_index(apply) != 0:
        return 1

    print("\nCargo.lock:")
    if opts["lock"]:
        if refresh_lock(apply) != 0:
            return 1
        if apply and verify_resolution() != 0:
            return 1
        if refresh_demo_lock(apply) != 0:
            return 1
    else:
        print("  skipped (--no-lock): refresh and commit Cargo.lock yourself, or every release lane's --locked build fails")

    print("\nTag order (dependencies first — a crate must be LIVE before its dependents tag; `v<ver>` is the lockstep slot, pushed by hand):")
    for i, t in enumerate(tag_order(pairs), 1):
        print(f"  {i}. {t}")

    print(
        "\nNext steps (all manual, and needing explicit sign-off; see docs/RELEASE-RUNBOOK-2026-10.md):\n"
        "  1. Review the diff and every release note (a scaffolded note is the CHANGELOG body,\n"
        "     not a written note).\n"
        "  2. python3 tools/finalize-release-check.py <pairs-file>   # read-only PASS/FAIL table\n"
        "  3. Run the full gate suite; commit, PR, merge.\n"
        "  4. tools/push-release-tags.sh --plan <pairs-file> — one tag at a time, in the order\n"
        "     above, verifying each reaches crates.io before the next (issue #933).\n"
        "  5. tools/prepare-release.py --verify <crate>=<version> ...\n"
        "  6. Confirm docs.rs built green for each (status.json)."
    )
    if not apply:
        print("\n(dry run — nothing was written; drop --check to apply)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
