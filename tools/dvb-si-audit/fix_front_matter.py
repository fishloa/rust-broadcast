#!/usr/bin/env python3
"""Correct `**Spec:** §X.Y` front matter in every docs/dvb_si/**/*.md.

Many descriptor files had a top-level `§6.2` reference (no subsection). The
spec numbers descriptors alphabetically (Component = §6.2.8, Network name =
§6.2.27, etc.) which makes tag-order guessing unreliable. This script:

  1. Parses the TOC from the pdftotext dump.
  2. Maps each descriptor's filename (after the `0xNN-` prefix, underscored
     to spaces) to a spec section by name.
  3. Rewrites the `**Spec:**` line in the md file.
  4. Leaves files without a resolvable section untouched.

Run:
    .venv/bin/python fix_front_matter.py
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

DOCS = Path(__file__).resolve().parents[2] / "docs/dvb_si"
DUMP = Path("/tmp/dvb-extract/full.txt")

TOC_LINE_RE = re.compile(
    r"^(\d+(?:\.\d+)+)\s+(.+?)\s*\.{2,}\s*(\d+)\s*$"
)
SPEC_LINE_RE = re.compile(
    r"^(\*\*Spec:\*\*\s*ETSI EN 300 468[^\n§]*)§\s*[0-9A-Za-z.]+\s*$",
    re.MULTILINE,
)


def normalise(s: str) -> str:
    """Lower, strip punctuation, collapse whitespace, drop 'descriptor' suffix."""
    s = s.lower().strip()
    s = re.sub(r"[^\w\s]", " ", s)
    s = re.sub(r"\s+", " ", s).strip()
    if s.endswith(" descriptor"):
        s = s[: -len(" descriptor")]
    return s


def parse_toc() -> dict[str, str]:
    """Return { normalised_title: section_id } from the pdftotext TOC.

    TOC lines look like:
        6.2.8         Component descriptor ................................................................. 59
    """
    if not DUMP.is_file():
        sys.exit(f"pdftotext dump not found: {DUMP}")

    by_name: dict[str, str] = {}
    for line in DUMP.read_text().splitlines():
        m = TOC_LINE_RE.match(line.strip())
        if not m:
            continue
        sec, title, _page = m.groups()
        by_name[normalise(title)] = sec
    return by_name


# Hand-curated overrides for files that don't match TOC by name.
# Keys are `filename stems` relative to docs/dvb_si; values are spec ids.
# - Annex-defined descriptors point to `D.X` / `G.X` / `H.X`.
# - External-ref descriptors point to their owning spec section fragment.
# - Files that don't carry a spec at all are set to `None` → spec line
#   is deleted rather than set to garbage.
OVERRIDES: dict[str, str | None] = {
    # Annex D — AC-3 family
    "0x6A-ac3": "D.1",
    "0x7A-enhanced_ac3": "D.2",
    "0x7F-extension/0x15-ac4": "D.5",
    # Annex G — DTS family
    "0x7B-dts": "G.1",
    "0x7F-extension/0x0E-dts_hd_audio_stream": "G.2",
    "0x7F-extension/0x0F-dts_neural": "G.3",
    # Annex H — AAC
    "0x7C-aac": "H",
    # Annex F — MHP XAIT
    "0x7D-xait_location": "F",
    "0x7F-extension/0x0C-xait_pid": "F",
    # External spec references (DVB-SI table 12 lists the tag; parsing is elsewhere)
    "0x71-service_identifier": "6.1",       # flagged as "external" in Table 12
    "0x73-default_authority": "6.1",
    "0x75-tva_id": "6.1",
    "0x77-time_slice_fec_identifier": "6.1",
    "0x78-ecm_repetition_rate": "6.1",
    "0x6F-application_signalling": "6.1",
    # §6.2.X aliases where normalisation failed
    "0x74-related_content": "6.2.26",      # actually related_content ≠ content
    "0x76-content_identifier": "6.2.11",   # content_identifier_descriptor
    "0x72-service_availability": "6.2.34",
    "0x63-partial_transport_stream": "6.2.29",  # was §7.2.1 (invalid)
    # Extension sub-tags with fuzzy mismatches
    "0x7F-extension/0x11-t2mi": "6.4.14",
    "0x7F-extension/0x14-ci_ancillary_data": "6.4.2",
    "0x7F-extension/0x17-s2x_satellite_delivery_system": "6.4.6.5",
    # Non-spec navigation files — remove the spec line
    "0x7F-extension/README": None,
    "descriptors/INDEX": None,
    "INDEX": None,
    "README": None,
    "tables/README": None,
    "tables/sat/README": None,
}


def filename_key(md_path: Path) -> str | None:
    """Extract a normalised descriptor/table key from the filename.

    descriptors/0x40-network_name.md -> 'network name'
    descriptors/0x7F-extension/0x14-ci_ancillary_data.md -> 'cid ancillary data'
    tables/sdt.md -> 'service description table'
    tables/sat/position_v2.md -> 'satellite position v2 info'
    """
    stem = md_path.stem
    # Strip 0xNN- prefix
    stem = re.sub(r"^0x[0-9A-Fa-f]+-", "", stem)
    # Underscores to spaces
    key = stem.replace("_", " ").strip()

    # Well-known table filenames
    table_aliases = {
        "pat": None,          # MPEG-2, not in 300 468
        "pmt": None,
        "cat": None,
        "nit": "network information table",
        "bat": "bouquet association table",
        "sdt": "service description table",
        "eit": "event information table",
        "tdt": "time and date table",
        "tot": "time offset table",
        "rst": "running status table",
        "st": "stuffing table",
        "dit": "discontinuity information table",
        "sit": "selection information table",
        # SAT family
        "position_v2": "satellite position v2 info",
        "position_v3": "satellite position v3 info",
        "cell_fragment": "cell fragment info",
        "time_association": "time association info",
        "beamhopping_time_plan": "beamhopping time plan info",
    }
    base = md_path.stem
    if base in table_aliases:
        alias = table_aliases[base]
        return alias
    return key


def extension_subtag_fallback(md_path: Path) -> str | None:
    """Extension 0x7F sub-tag lookup — these live under §6.4.X."""
    key = filename_key(md_path)
    return key


def main() -> int:
    toc = parse_toc()
    print(f"Parsed TOC: {len(toc)} entries")

    # Sanity prints
    for sample in ["component", "network name", "linkage", "service", "content"]:
        if sample in toc:
            print(f"  {sample!r:25s} -> §{toc[sample]}")

    md_files = sorted(DOCS.rglob("*.md"))
    updated = 0
    removed = 0
    skipped_no_match = 0
    skipped_no_spec = 0

    for md in md_files:
        text = md.read_text(encoding="utf-8")

        # Build a normalised override key matching the OVERRIDES dict
        rel = md.relative_to(DOCS).with_suffix("")
        rel_key = str(rel)  # e.g. "descriptors/0x6A-ac3"
        # Allow shorter forms: "0x6A-ac3", just the stem
        override_candidates = [
            rel_key,
            rel.as_posix(),
            md.stem,
        ]
        # strip leading "descriptors/" for convenience
        if rel_key.startswith("descriptors/"):
            override_candidates.append(rel_key[len("descriptors/"):])

        override = None
        for k in override_candidates:
            if k in OVERRIDES:
                override = OVERRIDES[k]
                break

        if override is None and any(k in OVERRIDES for k in override_candidates):
            # Override explicitly says None — strip the spec line
            new_text = SPEC_LINE_RE.sub("", text, count=1).lstrip("\n")
            if new_text != text:
                md.write_text(new_text, encoding="utf-8")
                removed += 1
            continue

        if override is not None:
            new_text, n = SPEC_LINE_RE.subn(
                lambda m: m.group(1) + f"§{override}", text, count=1
            )
            if n > 0 and new_text != text:
                md.write_text(new_text, encoding="utf-8")
                updated += 1
            continue

        if not SPEC_LINE_RE.search(text):
            skipped_no_spec += 1
            continue

        key = filename_key(md)
        if not key:
            continue

        normed = normalise(key)
        # Try several candidate keys
        candidates = [
            normed,
            normed + " descriptor",
            normed.replace(" ", ""),
        ]
        section = None
        for c in candidates:
            if c in toc:
                section = toc[c]
                break

        # Fuzzy fallback: match where TOC key contains normed
        if not section:
            for toc_key, sec in toc.items():
                if normed and (normed in toc_key or toc_key in normed):
                    section = sec
                    break

        if not section:
            skipped_no_match += 1
            continue

        new_text, n = SPEC_LINE_RE.subn(
            lambda m: m.group(1) + f"§{section}", text, count=1
        )
        if n > 0 and new_text != text:
            md.write_text(new_text, encoding="utf-8")
            updated += 1

    print(f"Updated via TOC + overrides: {updated}")
    print(f"Removed spec line (nav files): {removed}")
    print(f"Skipped (no spec line): {skipped_no_spec}")
    print(f"Skipped (no TOC match): {skipped_no_match}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
