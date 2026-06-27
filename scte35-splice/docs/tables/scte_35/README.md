# ANSI/SCTE 35 2023r1 — Digital Program Insertion Cueing Message (syntax reference)

> **✓ Accuracy-verified against the PDF — 2026-06-13.** The entire transcription
> was audited against the vendored `specs/ansi_scte_35_2023r1_dpi_cueing.pdf`
> (final arbiter), cross-checked against an independent Mistral-OCR rendering of
> the table pages (via BlazeDocs), with the PDF page opened directly wherever the
> two diverged. Every enum value↔name mapping, bit-field width/mnemonic, fixed
> length, and reserved range across ~30 tables (§8–§11) was confirmed; the §14
> sample messages were validated deterministically (each base64 decodes to its
> stated length and matches its hex + CRC-32). **Result: 0 discrepancies** — the
> md is faithful to the PDF. (OCR artifacts such as "Program Runner" for
> "Runover" were identified as OCR errors and the md's reading confirmed correct.)

**Provenance.** The canonical PDF is vendored at
`specs/ansi_scte_35_2023r1_dpi_cueing.pdf` and is the authoritative source.
The tables below were **hand-transcribed from that PDF on 2026-06-09**: the
SCTE page layout (ruled multi-column tables with an extra "Encrypted" column,
wrapped enumeration rows) is outside the table model of the ETSI-specific
geometry extractor in `tools/dvb-si-audit/`, which produced zero tables for
this document. Every syntax row (field name, bit width, mnemonic) and every
enumeration value was copied verbatim from the PDF page cited in each section
header. SCTE 35 is published by SCTE at no cost. Section numbers (§) are the
document's own; "PDF pp." are the printed page numbers, which coincide with
the PDF page numbers in this file.

Mnemonics are the MPEG conventions: `uimsbf` = unsigned integer, most
significant bit first; `bslbf` = bit string, left bit first; `rpchof` =
remainder polynomial coefficients, highest order first. Indentation inside
the Syntax column (rendered with `&nbsp;`) reproduces the nesting of the
spec's tables; the `if(...)`/`for(...)` lines are part of the normative
syntax and define the parse logic.
