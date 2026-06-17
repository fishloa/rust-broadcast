#!/usr/bin/env python3
"""Audit every docs/dvb_si/**/*.md against the ground-truth tables.

For each markdown file:
  1. Parse its front matter to get the spec section (`**Spec:** §X.Y`).
  2. Look up the ground-truth tables for that section from out/tables/.
  3. Count the rows in the markdown's own tables.
  4. Compare; flag any file where:
       - A referenced ground-truth table is missing from the md, OR
       - Row count in md is < 80% of ground-truth row count, OR
       - Red-flag prose appears ("selected", "see spec", "TODO", etc.)

Output: out/AUDIT.md (human-readable) + out/audit.json (machine-readable).

Run:
    .venv/bin/python audit.py
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

DOCS = Path(__file__).resolve().parents[2] / "dvb-si/docs"
GROUND = Path(__file__).resolve().parent / "out/tables"
OUT_DIR = Path(__file__).resolve().parent / "out"

SECTION_RE = re.compile(r"\*\*Spec:\*\*\s*ETSI EN 300 468[^\n§]*§\s*([0-9A-Za-z.]+)")
MD_TABLE_ROW_RE = re.compile(r"^\|")

RED_FLAGS = [
    "selected",
    "representative",
    "summary (abridged)",
    "see spec",
    "for full",
    "TODO",
    "abridged",
    "(subset)",
    "hand-polish",
]

# Files that legitimately carry no spec section.
NAV_FILES: set[str] = {
    "INDEX.md",
    "README.md",
    "overview.md",
    "tables/en_300_468/README.md",
    "descriptors/en_303_560/README.md",
    "descriptors/ts_102_727/README.md",
}


def load_ground_truth() -> dict[str, list[dict]]:
    """Return { section_id: [table_dict, ...] } sorted by table number."""
    by_section: dict[str, list[dict]] = {}
    for f in sorted(GROUND.glob("table_*.json")):
        t = json.loads(f.read_text())
        sec = t.get("section")
        if not sec:
            continue
        by_section.setdefault(sec, []).append(t)
    return by_section


def count_md_tables(md_text: str) -> list[int]:
    """Count rows of each Markdown table (| a | b | rows).

    Returns a list where each element is that table's body-row count
    (ignoring the separator line `|---|---|`)."""
    counts: list[int] = []
    current = 0
    in_table = False
    for line in md_text.splitlines():
        if MD_TABLE_ROW_RE.match(line):
            if set(line.strip()) <= set("|:- "):
                # separator row
                in_table = True
                continue
            current += 1
            in_table = True
        else:
            if in_table and current > 0:
                # Subtract 1 for the header row
                counts.append(max(0, current - 1))
                current = 0
                in_table = False
    if in_table and current > 0:
        counts.append(max(0, current - 1))
    return counts


def section_prefix(md_sec: str, target: str) -> bool:
    """True if md_sec is target or a sub-section of target.

    `6.2.19` matches tables in `6.2.19`, `6.2.19.1`, `6.2.19.4`, etc.
    """
    return md_sec == target or target.startswith(md_sec + ".")


def audit_file(md_path: Path, by_section: dict[str, list[dict]]) -> dict:
    text = md_path.read_text(encoding="utf-8")
    rel = str(md_path.relative_to(DOCS))
    entry: dict = {"path": rel, "issues": []}

    # Navigation/index/annex files don't point at a §-numbered SI entity.
    if (
        rel in NAV_FILES
        or rel.startswith("annexes/")
        or rel.startswith("text/iso_13818_6/")
        or rel.startswith("text/tr_101_211/")
        or rel.startswith("text/tr_101_290/")
        # MPEG-2 descriptors 0x02..0x12 — defined in ISO/IEC 13818-1, not 300 468
        or re.match(r"^descriptors/[^/]+/0x0[2-9A-Fa-f]-", rel)
        or re.match(r"^descriptors/[^/]+/0x1[0-2]-", rel)
        # MPEG-2 tables PAT/PMT/CAT
        or rel in {"tables/iso_13818_1/pat.md", "tables/iso_13818_1/pmt.md", "tables/iso_13818_1/cat.md"
    ):
        entry["section"] = None
        entry["kind"] = "navigation"
        return entry

    m = SECTION_RE.search(text)
    if not m:
        entry["section"] = None
        entry["issues"].append("no_spec_section_in_front_matter")
        return entry
    sec = m.group(1)
    entry["section"] = sec

    # Gather every ground-truth table whose section starts with `sec`.
    expected = [
        t
        for s, ts in by_section.items()
        if section_prefix(sec, s)
        for t in ts
    ]
    entry["expected_tables"] = [
        {"number": t["number"], "rows": len(t["rows"]), "section": t["section"]}
        for t in expected
    ]

    # Extract what the md actually renders as tables
    md_row_counts = count_md_tables(text)
    entry["md_table_row_counts"] = md_row_counts
    entry["md_total_rows"] = sum(md_row_counts)

    expected_total = sum(len(t["rows"]) for t in expected)
    entry["expected_total_rows"] = expected_total

    # Coverage %
    if expected_total == 0:
        entry["coverage_pct"] = None
    else:
        entry["coverage_pct"] = round(
            100.0 * entry["md_total_rows"] / expected_total, 1
        )

    # Missing-table check: if md has fewer distinct tables than ground truth
    if len(md_row_counts) < len(expected):
        entry["issues"].append(
            f"md_has_{len(md_row_counts)}_tables_vs_{len(expected)}_expected"
        )

    # Coverage threshold
    if entry["coverage_pct"] is not None and entry["coverage_pct"] < 80.0:
        entry["issues"].append(
            f"coverage_{entry['coverage_pct']:.0f}pct_below_80pct_threshold"
        )

    # Red flag prose
    low = text.lower()
    for flag in RED_FLAGS:
        if flag.lower() in low:
            entry["issues"].append(f"red_flag:{flag}")

    return entry


def main() -> int:
    if not GROUND.is_dir():
        sys.exit(f"Ground truth not found: {GROUND}. Run extract_tables.py first.")
    if not DOCS.is_dir():
        sys.exit(f"docs/dvb_si not found: {DOCS}")

    by_section = load_ground_truth()
    print(f"Loaded ground truth: {sum(len(v) for v in by_section.values())} tables across {len(by_section)} sections")

    md_files = sorted(DOCS.rglob("*.md"))
    print(f"Auditing {len(md_files)} markdown files")

    audit: list[dict] = []
    for md in md_files:
        audit.append(audit_file(md, by_section))

    # Stats
    nav = [a for a in audit if a.get("kind") == "navigation"]
    clean = [a for a in audit if a.get("kind") != "navigation" and not a["issues"]]
    flagged = [a for a in audit if a.get("kind") != "navigation" and a["issues"]]

    (OUT_DIR / "audit.json").write_text(
        json.dumps(audit, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )

    # Human-readable summary
    lines: list[str] = []
    lines.append(f"# DVB-SI markdown audit\n")
    lines.append(f"- Total files:  {len(audit)}")
    lines.append(f"- Navigation:   {len(nav)}  (INDEX/README/annexes/text — not audited)")
    lines.append(f"- Clean:        {len(clean)}")
    lines.append(f"- Flagged:      {len(flagged)}\n")

    # Top 30 worst files by coverage %
    with_cov = [
        a for a in flagged if a.get("coverage_pct") is not None
    ]
    with_cov.sort(key=lambda a: a["coverage_pct"])
    lines.append("## Worst coverage (lowest % of ground-truth rows present)\n")
    lines.append("| File | Section | Coverage | md rows | expected | Issues |")
    lines.append("|---|---|---|---|---|---|")
    for a in with_cov[:30]:
        lines.append(
            f"| `{a['path']}` | §{a['section']} | {a['coverage_pct']}% | "
            f"{a['md_total_rows']} | {a['expected_total_rows']} | "
            f"{', '.join(a['issues'][:3])} |"
        )
    lines.append("")

    # Files flagged ONLY for red-flag prose
    prose_only = [
        a for a in flagged
        if a.get("coverage_pct") is not None
        and a["coverage_pct"] >= 80.0
        and not any(i.startswith("md_has_") for i in a["issues"])
    ]
    lines.append(f"## Prose-only flags ({len(prose_only)} files)\n")
    lines.append("Files with adequate row coverage but suspicious prose ('selected', 'see spec', etc.).\n")
    lines.append("| File | Flags |")
    lines.append("|---|---|")
    for a in prose_only[:30]:
        flags = [i.split(":")[-1] for i in a["issues"] if i.startswith("red_flag:")]
        lines.append(f"| `{a['path']}` | {', '.join(flags)} |")
    lines.append("")

    # Files with missing tables
    missing = [a for a in flagged if any(i.startswith("md_has_") for i in a["issues"])]
    lines.append(f"## Files missing whole tables ({len(missing)})\n")
    lines.append("| File | Md tables | Expected | Coverage |")
    lines.append("|---|---|---|---|")
    for a in missing[:30]:
        lines.append(
            f"| `{a['path']}` | {len(a['md_table_row_counts'])} | "
            f"{len(a['expected_tables'])} | "
            f"{a.get('coverage_pct', 'n/a')}% |"
        )
    lines.append("")

    # No-front-matter-section files
    missing_sec = [a for a in audit if a.get("section") is None]
    lines.append(f"## Files with no `**Spec:**` front matter ({len(missing_sec)})\n")
    for a in missing_sec[:20]:
        lines.append(f"- `{a['path']}`")

    (OUT_DIR / "AUDIT.md").write_text("\n".join(lines), encoding="utf-8")

    print(f"\nAudit written to {OUT_DIR}/AUDIT.md + audit.json")
    print(f"  {len(clean)} clean / {len(flagged)} flagged")

    return 0


if __name__ == "__main__":
    sys.exit(main())
