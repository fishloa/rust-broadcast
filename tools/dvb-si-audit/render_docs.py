#!/usr/bin/env python3
"""Render one clean, complete reference markdown per ETSI spec.

Consumes the geometry-extracted table JSON (see extract_tables.py) and emits a
single self-contained document: a title, provenance line, a contents index, and
every `Table N` with its correctly-aligned syntax. No per-entity fragments, no
front-matter cruft, no parser-path guesses — just the spec, faithfully.

Run (after extract_tables.py has populated <out>):
    python render_docs.py --tables out/si/tables --title "ETSI EN 300 468 v1.19.1 — DVB SI" \
                          --pdf etsi_en_300_468_v01.19.01_dvb_si.pdf --o ../../dvb-si/docs/en_300_468.md
"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path


def num_key(n: str) -> tuple[int, str]:
    m = re.match(r"(\d+)([a-z]*)", n)
    return (int(m.group(1)), m.group(2)) if m else (0, n)


def slug(s: str) -> str:
    s = s.lower()
    s = re.sub(r"[^\w\s-]", "", s)
    return re.sub(r"\s+", "-", s).strip("-")


def md_table(rows: list[list[str]]) -> str:
    if not rows:
        return "_(no rows extracted)_"
    width = max(len(r) for r in rows)
    rows = [r + [""] * (width - len(r)) for r in rows]
    clean = lambda c: (c or "").replace("|", "\\|").strip()
    out = ["| " + " | ".join(clean(c) for c in rows[0]) + " |",
           "|" + "|".join("---" for _ in rows[0]) + "|"]
    for r in rows[1:]:
        out.append("| " + " | ".join(clean(c) for c in r) + " |")
    return "\n".join(out)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--tables", required=True, type=Path, help="<out>/tables dir of JSON")
    ap.add_argument("--title", required=True)
    ap.add_argument("--pdf", required=True, help="source PDF filename (for provenance)")
    ap.add_argument("--o", required=True, type=Path, help="output markdown path")
    args = ap.parse_args()

    tables = [json.loads(f.read_text()) for f in args.tables.glob("table_*.json")]
    tables.sort(key=lambda t: (t["first_page"], num_key(t["number"])))

    heads = [f"Table {t['number']} — {t['caption']}" for t in tables]
    out = [
        f"# {args.title}",
        "",
        f"Reference transcribed from the canonical PDF (`specs/{args.pdf}`) by the",
        "geometry-based extractor in `tools/dvb-si-audit/` — field rows aligned to",
        "their bit-widths by page geometry, reproduced verbatim. The PDF in `specs/`",
        "is the authoritative source.",
        "",
        "## Contents",
        "",
    ]
    out += [f"- [{h}](#{slug(h)})" for h in heads]
    out.append("")
    for t, h in zip(tables, heads):
        out.append(f"## {h}")
        sec = f"§{t['section']}, " if t.get("section") else ""
        out.append(f"_{sec}PDF pp. {t['first_page']}-{t['last_page']}_")
        out.append("")
        out.append(md_table(t["rows"]))
        out.append("")

    args.o.parent.mkdir(parents=True, exist_ok=True)
    args.o.write_text("\n".join(out) + "\n", encoding="utf-8")
    print(f"Wrote {args.o} ({len(tables)} tables)")
    return 0


if __name__ == "__main__":
    main()
