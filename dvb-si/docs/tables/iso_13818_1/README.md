# ISO/IEC 13818-1:2007 (MPEG-2 Systems) — adaptation field, PCR, and PSI section framing

> **✓ Field widths & mnemonics verified against the PDF — 2026-06-13.** The syntax-table field widths and mnemonic encodings (uimsbf/bslbf/tcimsbf/rpchof) in Tables 2-2, 2-6, 2-30 etc. were confirmed against the 2007 standard PDF, and the pseudocode equality operator was normalised to `==`. **Caveat:** the **Table 2-34 stream_type descriptions are condensed summaries** of the H.222.0 (2021) text, not verbatim — the *values* (0x00–0x35) are authoritative and verified, but for exact normative wording (profile/Annex qualifiers, Notes) consult the PDF. No split-cell repair was needed or performed in this pass.

**Provenance.** ISO/IEC 13818-1:2007 (identical text: ITU-T Rec. H.222.0
(05/2006)) is a paid ISO standard and cannot be vendored into this repository
— the PDF consulted for this transcription lives locally at
`specs/iso_iec_13818-1_2007_systems.pdf` and is deliberately **not** committed
(gitignored under `specs/iso_iec_*.pdf`). The syntax tables and semantics
below were hand-transcribed from that PDF on 2026-06-09. They are
cross-checkable against the live broadcast captures in `dvb-si/tests/fixtures/`
— both `.ts` fixtures carry adaptation fields with PCRs. PDF page numbers
cited below are the PDF file's own page indices (the printed spec page is
PDF page minus 12).
