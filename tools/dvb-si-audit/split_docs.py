#!/usr/bin/env python3
"""Split each monolithic spec transcription into one-table-per-file, reclassified
into the kind->spec tree, byte-for-byte preserving content. Idempotent.

A "monolith" = a `<crate>/docs/<kind>/<spec>/<spec>*.md` whose stem starts with
its own spec dir name (i.e. the whole-spec file that was moved but never split).
Each `## ` section becomes its own file; the kind dir is recomputed per section
(a descriptor section in an EN 300 468 monolith lands under descriptors/, etc.).
Content from a section's `## ` heading up to (not incl.) the next `## ` is copied
verbatim. The pre-`##` preamble becomes the spec dir's README.md. `## Contents`
sections are dropped (indexes are regenerated). A self-check asserts the rejoined
sections reproduce the original body.
"""
from __future__ import annotations
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CRATES = ["dvb-si", "dvb-t2mi", "dvb-scte35", "dvb-bbframe"]
KINDS = ["tables", "descriptors", "enums", "text"]

# Bad slug dirs -> normalized slug (rename/merge).
SLUG_FIX = {
    "ts_101162": "ts_101_162",
    "ts_101154": "ts_101_154",
    "ts_102366": "ts_102_366",
    "en_300706_teletext": "en_300_706",
    "en_300743_subtitling": "en_300_743",
}


def slugify(s: str) -> str:
    s = s.lower().strip()
    s = re.sub(r"[^\w\s-]", "", s)
    s = re.sub(r"[\s_]+", "-", s)
    return re.sub(r"-+", "-", s).strip("-")


def classify(caption: str, default_kind: str) -> str:
    c = caption.lower()
    if "descriptor" in c and "section" not in c:
        return "descriptors"
    if re.search(r"\b(coding|codes|type|types|value|values|mode|modes|"
                 r"assignment|assignments|status|allocation|kind)\b", c):
        return "enums"
    return default_kind


# "Table 11a — X", "Table 2-34 — X", "Table B.11 — X", "Table 4.1 — X", "§5.1.4 — X"
NUMRE = re.compile(r"^(?:Table|Annex)?\s*([A-Z]?[\d]+(?:[.\-][\d]+)?[a-z]?)\s*[—–-]\s*(.*)$")


def split_caption(heading: str):
    """Return (numeric_prefix_or_None, title) from a `## ` heading text."""
    h = heading.strip()
    m = NUMRE.match(h)
    if m:
        num = m.group(1).replace(".", "_")
        return num, m.group(2).strip()
    return None, h


def fix_slugs():
    for crate in CRATES:
        for kind in KINDS:
            base = ROOT / crate / "docs" / kind
            if not base.is_dir():
                continue
            for bad, good in SLUG_FIX.items():
                src = base / bad
                if not src.is_dir():
                    continue
                dst = base / good
                dst.mkdir(parents=True, exist_ok=True)
                for f in src.iterdir():
                    target = dst / f.name
                    if target.exists() and f.name == "README.md":
                        f.unlink()  # keep the canonical README, drop the dup
                        continue
                    f.rename(target)
                src.rmdir()
                print(f"  slug: {kind}/{bad} -> {kind}/{good}")


def find_monoliths():
    out = []
    for crate in CRATES:
        for kind in KINDS:
            base = ROOT / crate / "docs" / kind
            if not base.is_dir():
                continue
            for spec_dir in base.iterdir():
                if not spec_dir.is_dir():
                    continue
                spec = spec_dir.name
                for md in spec_dir.glob("*.md"):
                    if md.stem == spec or md.stem.startswith(spec + "_"):
                        out.append((crate, kind, spec, md))
    return out


def split_one(crate, default_kind, spec, md: Path):
    text = md.read_text(encoding="utf-8")
    parts = re.split(r"(?m)^(?=## )", text)
    preamble = parts[0] if not parts[0].startswith("## ") else ""
    sections = [p for p in parts if p.startswith("## ")]

    rejoined = []
    written = 0
    collisions = []
    for sec in sections:
        heading = sec.splitlines()[0][3:].strip()
        if heading.lower().startswith("contents"):
            continue  # dropped; index regenerated
        rejoined.append(sec)
        num, title = split_caption(heading)
        kind = classify(heading, default_kind)
        slug = slugify(title) or slugify(heading)
        name = f"{num}-{slug}.md" if num else f"{slug}.md"
        dest_dir = ROOT / crate / "docs" / kind / spec
        dest_dir.mkdir(parents=True, exist_ok=True)
        dest = dest_dir / name
        if dest.exists():
            collisions.append(name)
            continue  # canonical (monolith) already-present file wins; keep first
        dest.write_text(sec if sec.endswith("\n") else sec + "\n", encoding="utf-8")
        written += 1

    # byte-identity self-check: rejoined sections must equal original minus
    # preamble and minus the Contents section.
    orig_body = "".join(s for s in sections
                        if not s.splitlines()[0][3:].strip().lower().startswith("contents"))
    if "".join(rejoined) != orig_body:
        sys.exit(f"BYTE-IDENTITY FAIL splitting {md}")

    # preamble -> spec README (prepended once)
    if preamble.strip():
        readme = md.parent / "README.md"
        head = preamble.rstrip() + "\n"
        if readme.exists():
            existing = readme.read_text(encoding="utf-8")
            if preamble.strip() not in existing:
                readme.write_text(head + "\n" + existing, encoding="utf-8")
        else:
            readme.write_text(head, encoding="utf-8")

    md.unlink()
    print(f"  split {crate}/{default_kind}/{spec}/{md.name}: +{written} files"
          + (f", {len(collisions)} kept-existing" if collisions else ""))


def main():
    print("normalizing slugs...")
    fix_slugs()
    print("splitting monoliths...")
    monos = find_monoliths()
    if not monos:
        print("  (none — already split)")
    for crate, kind, spec, md in monos:
        split_one(crate, kind, spec, md)
    print("done.")


if __name__ == "__main__":
    main()
