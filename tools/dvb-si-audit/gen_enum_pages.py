#!/usr/bin/env python3
"""Generate a human Markdown page for any enum drift-guard `.toml` that has no
standalone spec coding-table sibling `.md`.

These cover coded fields whose values the spec defines INLINE inside a larger
syntax table (so no standalone table was transcribed to rename). The page is
rendered from the co-located `.toml` — the same spec-transcribed, drift-guarded
data — with a provenance header (the toml's own first-line spec citation). It
invents nothing: every value/label comes from the toml.
"""
from __future__ import annotations
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CRATES = ["dvb-si", "dvb-t2mi", "dvb-scte35", "dvb-bbframe"]


def parse(toml: str):
    cite = ""
    for line in toml.splitlines():
        if line.startswith("#") and cite == "":
            cite = line.lstrip("# ").strip()
            break
    rows, cur = [], {}
    for line in toml.splitlines():
        s = line.strip()
        if s == "[[entry]]":
            if cur:
                rows.append(cur)
            cur = {}
        elif "=" in s and not s.startswith("#"):
            k, v = s.split("=", 1)
            cur[k.strip()] = v.strip().strip('"')
    if cur:
        rows.append(cur)
    return cite, rows


def render(stem: str, cite: str, rows) -> str:
    title = stem.replace("_", " ")
    out = [f"# {title}", ""]
    if cite:
        out += [f"_{cite}_", ""]
    out += ["> Values rendered from the co-located drift-guard "
            f"[`{stem}.toml`](./{stem}.toml) — the spec defines these inline in a "
            "larger syntax table, so there is no standalone table to transcribe. "
            "The drift test keeps this list in lockstep with the Rust enum.", ""]
    out += ["| value | variant | spec meaning |", "|---|---|---|"]
    for r in rows:
        out.append(f"| {r.get('value','')} | `{r.get('variant','')}` | {r.get('spec','')} |")
    return "\n".join(out) + "\n"


def main():
    made = 0
    for crate in CRATES:
        base = ROOT / crate / "docs" / "enums"
        if not base.is_dir():
            continue
        for toml in base.rglob("*.toml"):
            md = toml.with_suffix(".md")
            if md.exists():
                continue
            cite, rows = parse(toml.read_text(encoding="utf-8"))
            md.write_text(render(toml.stem, cite, rows), encoding="utf-8")
            print(f"  generated {md.relative_to(ROOT)} ({len(rows)} values)")
            made += 1
    print(f"done — {made} pages generated")


if __name__ == "__main__":
    main()
