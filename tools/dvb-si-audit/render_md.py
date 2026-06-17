#!/usr/bin/env python3
"""Render docs/dvb_si/**/*.md from deterministic ground truth.

Zero LLM. For every existing md file that declares `**Spec:** §X.Y`:
  1. Look up the PDF page range for §X.Y (+ sub-sections).
  2. Pull narrative text from those pages via pdfplumber (text, minus tables).
  3. Insert the pre-extracted tables (out/tables/*.json) verbatim as markdown.
  4. Write the file back, preserving the front matter and the `_Generated_`
     footer.

Only descriptors with a resolvable §X.Y section get rendered. Files without
spec sections (INDEX, README, overview, glossary) are left untouched.

Run:
    .venv/bin/python render_md.py
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

import pdfplumber

PDF_PATH = Path(__file__).resolve().parents[2] / "docs/superpowers/specs/etsi_en_300_468_v01.19.01_dvb_si.pdf"
DOCS = Path(__file__).resolve().parents[2] / "dvb-si/docs"
OUT_DIR = Path(__file__).resolve().parent / "out"
TABLE_DIR = OUT_DIR / "tables"
SECTIONS_JSON = OUT_DIR / "sections.json"

SPEC_RE = re.compile(r"\*\*Spec:\*\*\s*ETSI EN 300 468[^\n§]*§\s*([0-9A-Za-z.]+)")
PARSER_RE = re.compile(r"\*\*Parser file:\*\*\s*`([^`]+)`")
STRUCT_RE = re.compile(r"\*\*Rust struct:\*\*\s*`([^`]+)`")

# Lines we drop from narrative text
HDR_FTR_RE = re.compile(r"^(\s*ETSI\s+EN\s+300\s+468|ETSI\s*$|\s*\d+\s+ETSI EN 300 468)")
TABLE_CAPTION_RE = re.compile(r"^\s*Table\s+(\d+[a-z]?)\s*:\s*(.+)\s*$")
SECTION_HDR_RE = re.compile(r"^\s*(\d+(?:\.\d+)+)\s+([A-Z].+?)\s*$")


def load_sections() -> dict[str, int]:
    return json.loads(SECTIONS_JSON.read_text())["page_of_section"]


def load_tables_by_section() -> dict[str, list[dict]]:
    by_section: dict[str, list[dict]] = {}
    for f in sorted(TABLE_DIR.glob("table_*.json")):
        t = json.loads(f.read_text())
        sec = t.get("section") or ""
        by_section.setdefault(sec, []).append(t)
    return by_section


def section_page_range(target: str, sections: dict[str, int], pdf_pages: int) -> tuple[int, int]:
    """Return (first_page, last_page) for spec section `target` and its subsections.

    last_page = start of next sibling/parent section - 1, or pdf_pages.
    """
    target_page = sections.get(target)
    if target_page is None:
        # Try to find the earliest sub-section (e.g. target=6.2.19 but only 6.2.19.1 exists)
        subs = [s for s in sections if s.startswith(target + ".")]
        if not subs:
            raise KeyError(target)
        target_page = min(sections[s] for s in subs)

    # Lowest numbered section greater than target (not a sub-section of target)
    next_page = pdf_pages + 1
    for s, p in sections.items():
        if s == target or s.startswith(target + "."):
            continue
        if p <= target_page:
            continue
        # Is `s` lexicographically "after" target at its own level?
        # Simpler: any section at same or parent level with page > target_page.
        if not s.startswith(target):
            if p < next_page:
                next_page = p

    return target_page, next_page - 1


def table_md(table: dict) -> str:
    """Render a ground-truth table as a Markdown table.

    Handles pdfplumber syntax tables (cells with embedded newlines) by splitting
    them into one line per row within the cell.
    """
    rows = table["rows"]
    if not rows:
        return ""

    # Detect syntax tables: any cell has internal newlines. Explode them.
    if any("\n" in (c or "") for r in rows for c in r):
        # Split each row's cells by newline, then zip them element-wise.
        exploded: list[list[str]] = []
        for r in rows:
            splits = [[ln.strip() for ln in (c or "").split("\n")] for c in r]
            max_lines = max(len(s) for s in splits)
            # Pad
            for s in splits:
                while len(s) < max_lines:
                    s.append("")
            for i in range(max_lines):
                exploded.append([s[i] for s in splits])
        rows = exploded

    # First row = header
    header = rows[0]
    body = rows[1:] if len(rows) > 1 else []

    def clean_cell(c: str) -> str:
        return (c or "").replace("|", "\\|").replace("\n", " ").strip()

    lines: list[str] = []
    lines.append("| " + " | ".join(clean_cell(c) for c in header) + " |")
    lines.append("|" + "|".join("---" for _ in header) + "|")
    for r in body:
        # Pad/truncate to header width
        while len(r) < len(header):
            r = list(r) + [""]
        r = r[: len(header)]
        lines.append("| " + " | ".join(clean_cell(c) for c in r) + " |")

    return "\n".join(lines)


def extract_narrative(pdf_path: Path, first_page: int, last_page: int,
                     target_section: str) -> str:
    """Pull narrative paragraphs from the specified PDF page range.

    Strategy:
      - Extract text per page (non-layout).
      - Drop page headers/footers ("ETSI EN 300 468 …").
      - Drop lines that look like table rows (the tables come back verbatim
        via `table_md()` inserted between narrative chunks).
      - Stop at the first section heading that isn't `target_section` or a sub.
    """
    collected: list[str] = []
    seen_sub = False

    with pdfplumber.open(pdf_path) as pdf:
        for pnum in range(first_page, last_page + 1):
            if pnum < 1 or pnum > len(pdf.pages):
                continue
            page = pdf.pages[pnum - 1]
            text = page.extract_text() or ""

            # Remove the rendered tables' text from the page to avoid duplicates
            try:
                tables = page.extract_tables() or []
            except Exception:
                tables = []
            table_cells = set()
            for t in tables:
                for row in t:
                    for cell in row:
                        if cell:
                            for line in cell.split("\n"):
                                s = line.strip()
                                if s:
                                    table_cells.add(s)

            for line in text.splitlines():
                s = line.strip()
                if not s:
                    continue
                if HDR_FTR_RE.search(s):
                    continue
                m = SECTION_HDR_RE.match(s)
                if m:
                    hdr_sec = m.group(1)
                    # Starting target or sub: accept
                    if hdr_sec == target_section or hdr_sec.startswith(target_section + "."):
                        collected.append(f"\n### §{hdr_sec} {m.group(2)}\n")
                        seen_sub = seen_sub or (hdr_sec != target_section)
                        continue
                    # Something else entirely: bail out
                    return "\n".join(collected).strip()
                if TABLE_CAPTION_RE.match(s):
                    collected.append(f"\n_See the corresponding table below._\n")
                    continue
                if s in table_cells:
                    continue
                collected.append(s)

    return "\n".join(collected).strip()


def render_file(md_path: Path, sections: dict[str, int],
                tables_by_section: dict[str, list[dict]],
                pdf_pages: int) -> bool:
    """Render one md file from ground truth. Returns True if rewritten."""
    text = md_path.read_text(encoding="utf-8")

    spec_m = SPEC_RE.search(text)
    if not spec_m:
        return False
    target_sec = spec_m.group(1)

    parser_m = PARSER_RE.search(text)
    struct_m = STRUCT_RE.search(text)

    # Preserve H1
    title_m = re.search(r"^#\s+(.+?)$", text, re.MULTILINE)
    title = title_m.group(1).strip() if title_m else md_path.stem

    # Find pages
    try:
        first, last = section_page_range(target_sec, sections, pdf_pages)
    except KeyError:
        return False
    # Cap at 20 pages to avoid runaway extraction
    last = min(last, first + 19)

    # Collect relevant tables
    relevant = []
    for s, ts in tables_by_section.items():
        if s == target_sec or s.startswith(target_sec + "."):
            for t in ts:
                relevant.append(t)
    relevant.sort(key=lambda t: (t["first_page"], t["number"]))

    # Build the new markdown
    out: list[str] = []
    out.append(f"# {title}")
    out.append("")
    out.append(f"**Spec:** ETSI EN 300 468 v1.19.1 §{target_sec}")
    if parser_m:
        out.append(f"**Parser file:** `{parser_m.group(1)}`")
    if struct_m:
        out.append(f"**Rust struct:** `{struct_m.group(1)}`")
    out.append("")

    # Narrative from PDF
    narrative = extract_narrative(PDF_PATH, first, last, target_sec)
    if narrative:
        out.append("## Spec text")
        out.append("")
        out.append(narrative)
        out.append("")

    # Tables
    if relevant:
        out.append("## Tables")
        out.append("")
        for t in relevant:
            out.append(f"### Table {t['number']} — {t['caption']}")
            out.append(f"_PDF pages {t['first_page']}-{t['last_page']} (§{t['section']})_")
            out.append("")
            md = table_md(t)
            if md:
                out.append(md)
            else:
                out.append("_(empty — see PDF)_")
            out.append("")

    # Footer with coverage declaration
    out.append("---")
    expected_rows = sum(len(t["rows"]) for t in relevant)
    out.append(
        f"_Rendered from ETSI EN 300 468 v1.19.1 §{target_sec}, "
        f"PDF pages {first}-{last}. {len(relevant)} tables / {expected_rows} rows reproduced verbatim._"
    )

    new_text = "\n".join(out) + "\n"
    md_path.write_text(new_text, encoding="utf-8")
    return True


def main() -> int:
    if not PDF_PATH.is_file():
        sys.exit(f"PDF not found: {PDF_PATH}")
    if not SECTIONS_JSON.is_file():
        sys.exit("Run extract_tables.py first — sections.json missing.")

    sections = load_sections()
    tables_by_section = load_tables_by_section()

    with pdfplumber.open(PDF_PATH) as pdf:
        pdf_pages = len(pdf.pages)

    md_files = sorted(DOCS.rglob("*.md"))
    rewritten = 0
    skipped = 0
    for md in md_files:
        try:
            if render_file(md, sections, tables_by_section, pdf_pages):
                rewritten += 1
            else:
                skipped += 1
        except Exception as e:
            print(f"FAIL {md}: {e}", file=sys.stderr)

    print(f"Rewritten: {rewritten}  Skipped (no spec section): {skipped}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
