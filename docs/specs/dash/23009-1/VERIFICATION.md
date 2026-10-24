# Verification record - ISO/IEC 23009-1:2022 (Ed.5) clauses 3, 4, 5.1-5.3.9 (public summary)

Date: 2026-10-24. The detailed record (every mismatch found and how it was resolved) is private, next to the verbatim transcription:
`private/specs-verified/dash/23009-1/VERIFICATION.md`. This public file keeps only what was verified, how, and the counts.

## What was verified

The verbatim transcription (private) of PDF pages 10-101 of `private/specs/iso_iec_23009-1_dash_2022_ed5.pdf` (printed pp. 2-93; printed = PDF - 8):
clause 3 (pp. 10-18), clause 4 (18-29, Tables 1-2), 5.1-5.2 (30-34), 5.3.1 (34-41, Table 3), 5.3.2 (41-47, Table 4), 5.3.3 (47-58, Tables 5-7),
5.3.4 (58-59, Table 8), 5.3.5 (59-70, Tables 9-12), 5.3.6 (70-71, Table 13), 5.3.7 (71-78, Table 14), 5.3.8 (78-79, Table 15), 5.3.9 (79-101, Tables 16-23).
The public files in this directory are original-wording notes written from that transcription; they were not separately re-verified against the PDF
beyond the spot checks in "Public-notes checks" below.

## How the private transcription was verified

| Step | Method | Result |
|---|---|---|
| Range conversion | `pdf2md convert <pdf> --pages 10-101 --engine hybrid --report` | exit code 0; every numeric token matched the PDF text layer; nothing flagged |
| Rendering and page comparison | `pdftoppm -r 110` for PDF pp. 10-101, each page read against the converted text | 92 of 92 pages compared |
| Independent re-check | 250 dpi render + `tesseract --psm 1`, per-page word-set comparison in both directions plus a punctuation-sensitive alignment | no word present in the transcription but absent on the page, none absent the other way, after discarding OCR noise |
| Table integrity | script: separator row and constant column count in every table | 23 tables (Tables 1-23), 0 malformed rows |
| Schema fragments | lxml structural comparison with the Ed.6 `DASH-MPD.xsd` | 39 types, 29 identical, 10 differ; see [`ED6-SCHEMA-DIFF.md`](ED6-SCHEMA-DIFF.md) |
| Crate index | `python3 tools/gen-crate-index.py --check` | up to date (this tree is docs-only) |

Counts of problems found in the raw text layer and fixed in the private file: every table needed rebuilding; 40 soft-hyphen artefacts; 26 split paragraphs re-joined; figures 1-3 absent (redrawn as ASCII, pp. 19-21).

## Where sources disagree

- Ed.5 print versus the Ed.6 schema files: [`ED6-SCHEMA-DIFF.md`](ED6-SCHEMA-DIFF.md) (10 of 39 types differ). Ed.6 prose (paywalled) was not read.
- Inside Ed.5: wrong cross-references, broken "subclause 0" fields, the `$Number$` start-number formula inconsistency of 5.3.9.5.3 and several typos. Each is described, with its page, in the "Transcription notes" section at the end of the relevant public file.

## Existing analysis files

`dash-23009-1-2022-aspect.md` and `dash-23009-1-2022-ids.md` (moved here earlier) agree with the verified rows: `@par` (Table 5, p. 42), `@width`/`@height`/`@sar` (Table 14, p. 64), `AdaptationSet@id`/`@group` (Table 5, p. 41), `Representation@id` (Table 9, p. 53).
The 2012-edition summary `specs/rules/dash-mpd-rules.md` is marked superseded; two of its rules contradicted Ed.5 (the `sidx` "shall be present" binding; `S@t` described as Period-relative).

## Gaps

- Clause 7.2.1 and Annex A (timing model; derivations for `@availabilityStartTime`, `@timeShiftBufferDepth`, `@minimumUpdatePeriod`, `@suggestedPresentationDelay`, ...): not covered. Within the covered range only 5.3.9.5.2, 5.3.9.5.3 and 5.3.9.6 give availability formulas.
- Clause 5.3.10 onward, 5.4-5.15, clauses 6-8, Annexes A-K (incl. Annex B, the normative schema, and Annex G examples): not covered.
- Figures 1-3 are redraws only.
- Cross-references printed as "subclause 0" are broken in the source and are not guessed.

## Public-notes checks

The leak check (no long verbatim runs of the ISO text in the public tree) is a scratch script, not committed: it flags any public line over 300 characters and any run of more than 3 consecutive lines
also present verbatim in `private/specs-md/iso_iec_23009-1_dash_2022_ed5.md`. Result at conversion time: 0 findings.
