# dvb-si-audit — spec table extraction

Deterministic, rule-based (no LLM) extraction of ETSI table syntax from the
vendored PDFs in [`../../specs/`](../../specs/), used to generate the per-crate
`docs/` spec references.

## Pipeline

1. **`extract_tables.py`** — geometry-based extraction. Reads any ETSI PDF and
   emits one JSON per `Table N` (+ optional markdown). Aligns each field row to
   its bit-width column by page geometry (table extent from horizontal ruling
   edges, columns from vertical-edge x-positions), so it is correct on both the
   tall-column-separator PDFs and the 2025 full-grid EN 300 468 PDF.
   ```bash
   ./setup.sh   # one-time: .venv + pdfplumber
   .venv/bin/python extract_tables.py --pdf ../../specs/etsi_en_300_468_v01.19.01_dvb_si.pdf --out out/si --md
   ```
2. **`render_docs.py`** — renders one clean reference markdown per spec from the
   extracted JSON (title, contents index, every table verbatim).
   ```bash
   .venv/bin/python render_docs.py --tables out/si/tables \
       --title "ETSI EN 300 468 v1.19.1 — DVB SI" \
       --pdf etsi_en_300_468_v01.19.01_dvb_si.pdf --o ../../dvb-si/docs/en_300_468.md
   ```

`render_md.py` / `audit.py` are the older EN 300 468-only helpers kept for
reference; `extract_tables.py` + `render_docs.py` supersede them.

## Deps

- Python 3.11+
- `pdfplumber` (pure Python, rule-based table parsing — no vision model)
