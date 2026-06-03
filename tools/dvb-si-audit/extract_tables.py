#!/usr/bin/env python3
"""Deterministic extraction of every `Table N` from an ETSI PDF.

Uses pdfplumber (rule-based PDF parsing, no LLM). Emits one JSON file per
`Table N: <caption>` to ``<out>/tables/table_<N>.json`` and, with ``--md``,
a markdown rendering of each table's syntax to ``<out>/md/table_<N>.md``.

Zero hallucination: tables come straight from PDF layout primitives.

Run:
    python extract_tables.py --pdf ../../specs/etsi_ts_102_006_v01.07.01_dvb_ssu.pdf \
                             --out out/ssu --md
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

import pdfplumber

# Matches a spec-section heading anywhere in the text layout, e.g.:
#   "5.2.4 Event Information Table" / "9.4.1 UNT Syntax"
SECTION_HDR_RE = re.compile(r"^\s*(\d+(?:\.\d+)+)\s+[A-Z]", re.MULTILINE)

# Matches a table caption, e.g. "Table 11: Syntax of the Update Notification Section"
TABLE_CAPTION_RE = re.compile(r"Table\s+(\d+[a-z]?)\s*:\s*(.+?)(?=\n|$)")


def table_md(table: dict) -> str:
    """Render a ground-truth table as Markdown, exploding syntax cells.

    ETSI syntax tables encode one field per line inside a single cell;
    split those into one row per field. (Shared logic with render_md.py.)
    """
    rows = table["rows"]
    if not rows:
        return ""
    if any("\n" in (c or "") for r in rows for c in r):
        exploded: list[list[str]] = []
        for r in rows:
            splits = [[ln.strip() for ln in (c or "").split("\n")] for c in r]
            max_lines = max(len(s) for s in splits)
            for s in splits:
                while len(s) < max_lines:
                    s.append("")
            for i in range(max_lines):
                exploded.append([s[i] for s in splits])
        rows = exploded
    header = rows[0]
    body = rows[1:] if len(rows) > 1 else []

    def clean(c: str) -> str:
        return (c or "").replace("|", "\\|").replace("\n", " ").strip()

    lines = ["| " + " | ".join(clean(c) for c in header) + " |",
             "|" + "|".join("---" for _ in header) + "|"]
    for r in body:
        r = list(r)
        while len(r) < len(header):
            r.append("")
        r = r[: len(header)]
        lines.append("| " + " | ".join(clean(c) for c in r) + " |")
    return "\n".join(lines)


def parse_args() -> argparse.Namespace:
    ap = argparse.ArgumentParser(description="Extract ETSI tables from a PDF (rule-based, no LLM).")
    ap.add_argument("--pdf", required=True, type=Path, help="path to the ETSI PDF")
    ap.add_argument("--out", default=Path("out"), type=Path, help="output dir (default: ./out)")
    ap.add_argument("--md", action="store_true", help="also emit per-table markdown to <out>/md/")
    return ap.parse_args()


def main() -> int:
    args = parse_args()
    if not args.pdf.is_file():
        sys.exit(f"PDF not found: {args.pdf}")

    table_dir = args.out / "tables"
    table_dir.mkdir(parents=True, exist_ok=True)

    sections: dict[str, int] = {}
    tables: list[dict] = []
    current_section: str | None = None

    with pdfplumber.open(args.pdf) as pdf:
        for page_num, page in enumerate(pdf.pages, start=1):
            text = page.extract_text() or ""
            for m in SECTION_HDR_RE.finditer(text):
                sections.setdefault(m.group(1), page_num)
                current_section = m.group(1)

            captions = [
                {"number": m.group(1), "caption": m.group(2).strip(), "pos": m.start()}
                for m in TABLE_CAPTION_RE.finditer(text)
            ]
            extracted = page.extract_tables() or []
            num_captioned = min(len(captions), len(extracted))

            for i in range(num_captioned):
                cap = captions[i]
                rows = [[(c or "").strip() for c in row] for row in extracted[i]]
                tables.append({
                    "number": cap["number"], "caption": cap["caption"],
                    "first_page": page_num, "last_page": page_num,
                    "section": current_section, "rows": rows,
                })

            trailing = extracted[num_captioned:]
            if trailing and tables:
                tgt = tables[-1]
                for raw_rows in trailing:
                    extra = [[(c or "").strip() for c in row] for row in raw_rows]
                    if extra and tgt["rows"] and extra[0] == tgt["rows"][0]:
                        extra = extra[1:]
                    tgt["rows"].extend(extra)
                    tgt["last_page"] = page_num

    for t in tables:
        safe = t["number"].replace(".", "_")
        (table_dir / f"table_{safe}.json").write_text(
            json.dumps(t, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

    (args.out / "sections.json").write_text(
        json.dumps({"page_of_section": sections}, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8")

    if args.md:
        md_dir = args.out / "md"
        md_dir.mkdir(parents=True, exist_ok=True)
        for t in tables:
            safe = t["number"].replace(".", "_")
            body = table_md(t)
            doc = (f"### Table {t['number']} — {t['caption']}\n"
                   f"_§{t['section']}, PDF pages {t['first_page']}-{t['last_page']}_\n\n"
                   f"{body}\n")
            (md_dir / f"table_{safe}.md").write_text(doc, encoding="utf-8")

    print(f"Extracted {len(tables)} tables across {len(sections)} sections -> {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
