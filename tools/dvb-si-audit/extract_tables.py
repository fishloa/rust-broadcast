#!/usr/bin/env python3
"""Geometry-based extraction of every `Table N` from an ETSI PDF.

Rule-based, no LLM, zero hallucination. ETSI syntax tables pack one field per
line inside a single ruled cell, and brace/`for`/`if` rows carry no bit-width —
so the naive "split the cell on newlines and zip the columns" approach shifts
the bit-width column by a row (the bug that produced garbled docs).

This extractor aligns by GEOMETRY instead:
  1. `find_tables` (lines strategy) gives each table's bbox + the x of every
     vertical ruling line -> the column boundaries.
  2. `extract_words` gives every word's (x0, top) box. Words are clustered into
     visual rows by their `top` y-coordinate, then binned into columns by `x0`.
  3. A field and its bit-width share a `top`, so they land on the same row;
     a `{` / `}` / `for(...)` row simply has no word in the bits column.

Emits one JSON per `Table N` to ``<out>/tables/`` and, with ``--md``, an
aligned markdown rendering to ``<out>/md/``.

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

SECTION_HDR_RE = re.compile(r"^\s*(\d+(?:\.\d+)+)\s+[A-Z]", re.MULTILINE)
TABLE_CAPTION_RE = re.compile(r"Table\s+(\d+[a-z]?)\s*:\s*(.+?)(?=\n|$)")
# Page header/footer noise to drop from extracted rows.
NOISE_RE = re.compile(r"^(ETSI|\d+\s+ETSI|ETSI\s+(EN|TS)\s)")

# Two words belong to the same visual row when their `top` differs by < this (pt).
ROW_TOL = 3.0
# A vertical edge must be at least this tall to count as a table column rule
# (filters out glyph strokes and header-only sub-rules).
MIN_VEDGE_H = 15.0


def _merge_close(xs: list[float], tol: float = 3.0) -> list[float]:
    """Collapse x-values within `tol` of each other to a single boundary."""
    merged: list[float] = []
    for x in sorted(xs):
        if merged and x - merged[-1] <= tol:
            continue
        merged.append(x)
    return merged


def detect_tables(page) -> list[dict]:
    """Detect tables by geometry — works across both ETSI PDF rendering styles.

    Older PDFs (TS 102 006/EN 301 192/TS 102 323) draw tall column separators
    with no inner horizontal rules; the 2025 EN 300 468 PDF draws a full
    per-cell grid (many short edges). Both are handled by:
      * y-extent: cluster HORIZONTAL-edge y's into bands (a >25pt gap starts a
        new table); each band is one table's vertical span.
      * columns: within a band, histogram VERTICAL-edge x weighted by the
        height each edge contributes inside the band; x's whose stacked height
        covers a meaningful fraction of the band are the column boundaries.
    `find_tables` is not used — it needs both rules and misses the no-inner-rule
    style entirely.
    """
    h_y = sorted({round(e["top"], 1) for e in page.edges if e["orientation"] == "h"})
    if len(h_y) < 2:
        return []
    bands: list[tuple[float, float]] = []
    start = prev = h_y[0]
    for y in h_y[1:]:
        if y - prev > 25:
            bands.append((start, prev))
            start = y
        prev = y
    bands.append((start, prev))

    regions = []
    for top, bottom in bands:
        if bottom - top < 8:  # a stray rule, not a table
            continue
        vx: dict[float, float] = {}
        for e in page.edges:
            if e["orientation"] != "v":
                continue
            overlap = min(e["bottom"], bottom) - max(e["top"], top)
            if overlap > 0:
                vx[round(e["x0"], 1)] = vx.get(round(e["x0"], 1), 0.0) + overlap
        thr = max(MIN_VEDGE_H, (bottom - top) * 0.25)
        bounds = _merge_close([x for x, ht in vx.items() if ht >= thr])
        if len(bounds) >= 2:
            regions.append({"bounds": bounds, "top": top - 2, "bottom": bottom + 2})
    regions.sort(key=lambda r: r["top"])
    return regions


def col_index(x: float, bounds: list[float]) -> int:
    """Column index for a word at x, given sorted column boundaries."""
    for i in range(len(bounds) - 1):
        # left-inclusive; small slack so a glyph straddling the rule lands left.
        if x < bounds[i + 1] - 0.5:
            return i
    return len(bounds) - 2


def table_rows(page, region: dict) -> list[list[str]]:
    """Reconstruct visual rows of `region` by clustering words on y, binning on x."""
    bounds = region["bounds"]
    ncol = max(1, len(bounds) - 1)
    top, bottom = region["top"], region["bottom"]
    words = [
        w
        for w in page.extract_words(use_text_flow=False, keep_blank_chars=False)
        if w["top"] >= top - 1 and w["bottom"] <= bottom + 1
        and w["x0"] >= bounds[0] - 1 and w["x0"] < bounds[-1]
    ]
    words.sort(key=lambda w: (w["top"], w["x0"]))

    rows: list[list[str]] = []
    cur: list[dict] = []
    cur_top: float | None = None
    for w in words:
        if cur_top is None or abs(w["top"] - cur_top) <= ROW_TOL:
            cur.append(w)
            cur_top = w["top"] if cur_top is None else cur_top
        else:
            rows.append(_bin_row(cur, bounds, ncol))
            cur, cur_top = [w], w["top"]
    if cur:
        rows.append(_bin_row(cur, bounds, ncol))
    # Drop page header/footer rows that crept into the bbox.
    return [r for r in rows if not NOISE_RE.match("".join(r).strip())]


def _bin_row(words: list[dict], bounds: list[float], ncol: int) -> list[str]:
    cells = [""] * ncol
    for w in sorted(words, key=lambda w: w["x0"]):
        ci = col_index(w["x0"], bounds)
        cells[ci] = (cells[ci] + " " + w["text"]).strip()
    return cells


def parse_args() -> argparse.Namespace:
    ap = argparse.ArgumentParser(description="Geometry-based ETSI table extraction (no LLM).")
    ap.add_argument("--pdf", required=True, type=Path)
    ap.add_argument("--out", default=Path("out"), type=Path)
    ap.add_argument("--md", action="store_true", help="also emit per-table markdown")
    ap.add_argument("--page", type=int, help="debug: dump rows for one (1-based) page and exit")
    return ap.parse_args()


def md_table(rows: list[list[str]]) -> str:
    if not rows:
        return ""
    width = max(len(r) for r in rows)
    rows = [r + [""] * (width - len(r)) for r in rows]
    header = rows[0]
    clean = lambda c: c.replace("|", "\\|").strip()
    out = ["| " + " | ".join(clean(c) for c in header) + " |",
           "|" + "|".join("---" for _ in header) + "|"]
    for r in rows[1:]:
        out.append("| " + " | ".join(clean(c) for c in r) + " |")
    return "\n".join(out)


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
        page_iter = (
            [(args.page, pdf.pages[args.page - 1])]
            if args.page
            else list(enumerate(pdf.pages, start=1))
        )
        for page_num, page in page_iter:
            text = page.extract_text() or ""
            for m in SECTION_HDR_RE.finditer(text):
                sections.setdefault(m.group(1), page_num)
                current_section = m.group(1)
            captions = [
                (m.group(1), m.group(2).strip())
                for m in TABLE_CAPTION_RE.finditer(text)
            ]
            found = detect_tables(page)
            # Pair tables (top-to-bottom) with captions (in reading order); a
            # table with no caption continues the previous captioned table.
            cap_i = 0
            for t in found:
                rows = table_rows(page, t)
                if not rows:
                    continue
                if cap_i < len(captions):
                    num, cap = captions[cap_i]
                    cap_i += 1
                    tables.append({
                        "number": num, "caption": cap, "section": current_section,
                        "first_page": page_num, "last_page": page_num, "rows": rows,
                    })
                elif tables:  # continuation
                    tables[-1]["rows"].extend(rows)
                    tables[-1]["last_page"] = page_num

    if args.page:
        for t in tables:
            print(f"\n### Table {t['number']}: {t['caption']}  (§{t['section']})")
            print(md_table(t["rows"]))
        return 0

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
            doc = (f"### Table {t['number']} — {t['caption']}\n"
                   f"_§{t['section']}, PDF pp. {t['first_page']}-{t['last_page']}_\n\n"
                   f"{md_table(t['rows'])}\n")
            (md_dir / f"table_{safe}.md").write_text(doc, encoding="utf-8")

    print(f"Extracted {len(tables)} tables across {len(sections)} sections -> {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
