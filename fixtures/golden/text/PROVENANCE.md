# PROVENANCE


## `text/iso6937_combining.tsv`

An independent oracle for the ETSI EN 300 468 V1.19.1 (2025-02) Annex A
Figure A.1 default-Latin (ISO/IEC 6937 superset) combining-prefix row
(0xC1..=0xCF). Read by
`dvb-si/tests/it/text_iso6937_table.rs`.

Contents: one row per (non-spacing prefix, base byte 0x20..0x7E) pair — the
13 defined prefixes of Figure A.1 (0xC0/0xC9/0xCC are grey) times the 95
printable ASCII base bytes. Columns are `prefix`, `base` (hex) and
`codepoints` (space-separated `U+XXXX`).

The **set** of precomposed pairs is transcribed from ISO/IEC 6937's
diacritic/letter combination table, which Figure A.1's C-row adopts. The
**accents themselves are not read from the Rust table**: each supported
pair's expected glyph was computed with Unicode Normalization Form C,
`unicodedata.normalize('NFC', base + mark)` using the Figure A.1 C-row
combining codepoint for that prefix, so a mistyped `combine` arm is a
mismatch against this file. A pair outside the ISO 6937 set carries the
documented fallback, base followed by the combining mark, which is what
`decode_iso_6937` emits when there is no precomposed form.

Regenerated once with Python 3 (standard library only) from the transcription
at `private/specs-verified/dvb/dvb-si/tables/en_300_468/figure-a1-character-code-table-00-default-latin-alphabet.md`:

    python3 .delegate/scratch/gen_iso6937.py > fixtures/golden/text/iso6937_combining.tsv

The generator is kept out of the tree because it is not built or read by any
test; the committed table is the oracle.
