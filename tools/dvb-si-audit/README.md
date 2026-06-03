# dvb-si-audit

Deterministic table extraction + audit tooling for the `docs/dvb_si/` markdown tree.

Two stages:

1. **`extract_tables.py`** — reads `docs/superpowers/specs/etsi_en_300_468_v01.19.01_dvb_si.pdf`
   with `pdfplumber` and emits ground-truth tables (one JSON file per Table N) to `out/tables/`.
   Rule-based, zero hallucination risk.

2. **`audit.py`** — compares each `docs/dvb_si/**/*.md` against the ground truth and reports
   missing rows / abridgements.

Run:

```bash
./setup.sh            # creates .venv, installs deps
.venv/bin/python extract_tables.py
.venv/bin/python audit.py
```

## Deps

- Python 3.11+
- `pdfplumber` (pure Python, no vision model needed for rule-based table parsing)
