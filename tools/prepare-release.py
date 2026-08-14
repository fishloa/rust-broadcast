#!/usr/bin/env python3
"""Prepare a release: bump versions, cut CHANGELOGs, scaffold release notes.

Usage:
    tools/prepare-release.py <crate>=<version> [<crate>=<version> ...]
    tools/prepare-release.py --check <crate>=<version> [...]     # dry run
    tools/prepare-release.py --verify <crate>=<version> [...]    # post-publish

Does the mechanical, error-prone half of `docs/RELEASE-DOCS.md` so it is not
retyped by hand each time:

1. **Version bump** — sets `version = "X.Y.Z"` in each crate's `Cargo.toml`.
2. **Sibling dep refs** — updates every OTHER workspace crate that depends on a
   bumped crate to the new caret version, so no published bucket ends up
   spanning two epochs (the #858 rule).
3. **CHANGELOG cut** — `## [Unreleased]` becomes `## [X.Y.Z] - YYYY-MM-DD` and
   a fresh empty `## [Unreleased]` goes back on top.
4. **Release note** — scaffolds `docs/release-notes/<crate>-<version>.md` from
   the CHANGELOG section just cut, if one does not already exist.
5. **Tag order** — prints the dependency-correct tag order, so a crate is never
   tagged before something it depends on is live.

It does NOT tag, push, or publish. Those stay manual and need explicit
sign-off; `tools/push-release-tags.sh` pushes tags one at a time afterwards.

`--verify` checks each crate is live at the expected version via
`tools/crates-io.py`, which is the only supported way to ask crates.io (it
sends the User-Agent crates.io requires, and never reports a failed request as
"not published").
"""

from __future__ import annotations

import datetime as _dt
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
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
    """The caret-range string a sibling should depend on: major.minor for 0.x,
    major for >=1.0 — matching how this workspace already writes them."""
    parts = version.split(".")
    if parts[0] == "0":
        return f"{parts[0]}.{parts[1]}"
    return parts[0]


def update_sibling_refs(crate: str, new: str, apply: bool) -> list[str]:
    """Point every other workspace crate's dependency on `crate` at `new`."""
    out: list[str] = []
    want = caret(new)
    dep_re = re.compile(
        rf'(^{re.escape(crate)}\s*=\s*\{{[^}}]*?version\s*=\s*)"([^"]+)"',
        re.M,
    )
    for manifest in sorted(ROOT.glob("*/Cargo.toml")):
        if manifest.parent.name == crate:
            continue
        text = manifest.read_text()
        m = dep_re.search(text)
        if not m or m.group(2) == want:
            continue
        if apply:
            manifest.write_text(dep_re.sub(lambda mm: f'{mm.group(1)}"{want}"', text, count=1))
        out.append(f"  {manifest.parent.name}: dep {crate} {m.group(2)} -> {want}")
    return out


def cut_changelog(crate: str, new: str, apply: bool) -> tuple[str, str]:
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
    if not body.strip():
        return (f"  {crate}: [Unreleased] is EMPTY — nothing to release", "")

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


def dependency_order(crates: list[str]) -> list[str]:
    """Order crates so a dependency is tagged before its dependents."""
    deps: dict[str, set[str]] = {}
    for c in crates:
        text = crate_manifest(c).read_text()
        deps[c] = {o for o in crates if o != c and re.search(rf"^{re.escape(o)}\s*=", text, re.M)}
    ordered: list[str] = []
    remaining = dict(deps)
    while remaining:
        ready = sorted(c for c, d in remaining.items() if not (d - set(ordered)))
        if not ready:  # cycle — fall back to input order rather than hang
            ordered.extend(sorted(remaining))
            break
        ordered.extend(ready)
        for c in ready:
            remaining.pop(c)
    return ordered


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


def main(argv: list[str]) -> int:
    args = argv[1:]
    if not args:
        print(__doc__, file=sys.stderr)
        return 2

    mode = "apply"
    if args[0] == "--check":
        mode, args = "check", args[1:]
    elif args[0] == "--verify":
        mode, args = "verify", args[1:]
    elif args[0] == "--restamp":
        mode, args = "restamp", args[1:]
    elif args[0] == "--restamp-check":
        mode, args = "restamp-check", args[1:]

    pairs: list[tuple[str, str]] = []
    for a in args:
        if "=" not in a:
            raise SystemExit(f"bad argument {a!r}, expected <crate>=<version>")
        c, v = a.split("=", 1)
        pairs.append((c, v))

    if mode == "verify":
        return verify(pairs)

    if mode in ("restamp", "restamp-check"):
        doing = mode == "restamp"
        print(f"{'RESTAMPING' if doing else 'DRY RUN --'} release dates to {TODAY}\n")
        return restamp_dates(pairs, doing)

    apply = mode == "apply"
    print(f"{'APPLYING' if apply else 'DRY RUN —'} release prep for {len(pairs)} crate(s)\n")

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

    print("\nCHANGELOGs:")
    bodies: dict[str, str] = {}
    for c, v in pairs:
        status, body = cut_changelog(c, v, apply)
        print(status)
        bodies[c] = body

    print("\nRelease notes:")
    for c, v in pairs:
        print(scaffold_release_note(c, v, bodies.get(c, ""), apply))

    print("\nTag order (dependencies first — a crate must be LIVE before its dependents tag):")
    for i, c in enumerate(dependency_order([c for c, _ in pairs]), 1):
        v = dict(pairs)[c]
        print(f"  {i}. {c}-v{v}")

    print(
        "\nNext steps (all manual, and needing explicit sign-off):\n"
        "  1. Review and EDIT every scaffolded release note — the scaffold is the\n"
        "     CHANGELOG body, not a written note.\n"
        "  2. Update the root README coverage table for any NEW crate.\n"
        "  3. Run the full gate suite.\n"
        "  4. Commit, PR, merge.\n"
        "  5. tools/push-release-tags.sh — pushes tags ONE AT A TIME, in the order\n"
        "     above, verifying each reaches crates.io before the next (issue #933).\n"
        "  6. tools/prepare-release.py --verify <crate>=<version> ...\n"
        "  7. Confirm docs.rs built green for each."
    )
    if not apply:
        print("\n(dry run — nothing was written; drop --check to apply)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
