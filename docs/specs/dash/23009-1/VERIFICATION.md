# Verification record - ISO/IEC 23009-1:2022 (Ed.5) clauses 3, 4, 5.1-5.3.9

Date: 2026-10-24. Source: `private/specs/iso_iec_23009-1_dash_2022_ed5.pdf` (private submodule, pinned sha `25ba13a5`), PDF pages 10-101
(printed pages 2-93; printed = PDF - 8). Text-layer starting point: `private/specs-md/iso_iec_23009-1_dash_2022_ed5.md` was *not* used
as the working copy (it is a whole-document conversion); a fresh range conversion was made instead (below).

## 1. Sources and commands

| Step | Command / method | Result |
|---|---|---|
| Range conversion | `cd ~/Projects/pdf2md && uv run pdf2md convert <pdf> -o hyb_10-101.md --pages 10-101 --engine hybrid --report` | **exit code 0**, stderr empty: every numeric token in PDF pp. 10-101 matched the embedded text layer; no flagged token to resolve |
| Rendering | `pdftoppm -r 110 -f 10 -l 101 -png <pdf> p` | 92 PNGs, one per PDF page |
| Page-by-page comparison | each rendered page was opened and read against the converted text of the same page | **all 92 pages (PDF 10-101) compared**, none skipped |
| Independent re-check | `pdftoppm -r 250` + `tesseract --psm 1` per page; per-page word-set comparison in both directions and a punctuation-sensitive sequence alignment against the final transcription | after discarding OCR noise (broken monospace identifiers, subscripts, merged words, running header/footer, figures), **no word present in the transcription but absent from the page, and no page word absent from the transcription**; remaining punctuation differences were all OCR misreads or tokenisation of markup (each inspected) |
| Table integrity | script: every table block has a separator row and the same column count on every row | 0 malformed rows (23 tables, Tables 1-23) |
| XML fragments | lxml structural comparison against `DASH-MPD.xsd` (Ed.6) | see [`ED6-SCHEMA-DIFF.md`](ED6-SCHEMA-DIFF.md) |
| Crate index | `python3 tools/gen-crate-index.py --check` | `crate index up to date` (docs-only tree under `docs/specs/`, not a crate; `cpix/` and `st2110/` are likewise unregistered) |

The text layer was the source of the words; the rendered page was the authority for everything else. The pdf2md exit-0 result covers
digits only; the OCR cross-check is what covers punctuation and wording.

## 2. Pages rendered and compared

PDF pages 10-101, all of them: 10-18 (clause 3), 18-29 (clause 4, incl. Tables 1-2 on 26-29), 30-34 (5.1-5.2), 34-41 (5.3.1, Table 3 on
35-40), 41-47 (5.3.2, Table 4 on 43-45), 47-58 (5.3.3, Tables 5-7 on 49-57), 58-59 (5.3.4, Table 8), 59-70 (5.3.5, Tables 9-12 on 61-68),
70-71 (5.3.6, Table 13), 71-78 (5.3.7, Table 14 on 72-77), 78-79 (5.3.8, Table 15), 79-100 (5.3.9, Tables 16-23 on 81-100). Page 101 holds
the end of 5.3.9.7 and the start of 5.3.10; only the part up to the 5.3.10 heading is transcribed.

## 3. Mismatches found between the text-layer conversion and the rendered page, and how each was resolved

1. **Every table (Tables 1-23) was mangled in the text layer** - GFM fragments, one-row-per-line splits, repeated header fragments, truncated
   identifiers in the name column (`@mediaPresentationDuratio`+`n`, `@suggestedPresentationDel`+`ay`, `@initializationPrincipa`+`l`,
   `CommonAttributesElem`+`ents`, `AudioChannelConfigur`+`ation`), Use-cell content bleeding into the description (`default: ... false,`), page-split
   rows. **Resolved:** every row of every table was rebuilt cell by cell against the render (hand-typed for the rows with merged/odd cells, parsed
   then checked for the others); Use codes and defaults read from the render.
2. **Punctuation displaced or lost by the text layer.** Examples (all fixed to the render): clause 4.3 "live content, where ... end-to-end latency"
   (comma had moved to "end-to-end, latency"); Table 5 `@segmentAlignment` "Representations, X and Y, within" and `@subsegmentAlignment`
   "set to 'true', all following" (commas had migrated into the "default :,," cell); Table 21 `$Number$` "Segment, if $SubNumber$"; Table 3
   `@type` "(@type="static"), all Segments" (and the stray words "static available"); Table 12 `@bufferTime` row; "Period′s" and similar.
3. **Sub/superscripts flattened:** I<sub>SAP</sub>, T<sub>SAP</sub>, T<sub>DEC</sub>, T<sub>EPT</sub>, T<sub>PTF</sub>, I<sub>SAU</sub>, T<sub>E</sub>/T<sub>L</sub>,
   k<sub>start</sub>, Number<sub>Start</sub>, S<sub>i,j</sub>, B<sub>i(k)</sub>, footnote references `[3] [8] [9] [10] [21]-[25]` restored from the render.
4. **40 U+FFFE (soft-hyphen) artefacts** in the text layer ("On-Demand", "end-to-end") replaced by `-` after checking each against the render.
5. **Spurious spaces** ("( for example", " :", " ,") from the text layer removed where the render shows none; **missing spaces** restored where the render
   has one ("PeriodStarttime" -> "PeriodStart time"). Spaces the render genuinely lacks were kept ("subclause5.3.2", "ofIETF RFC 6381:2011",
   "6.3.2.4shall").
6. **Figures 1, 2, 3 absent** from the text layer: described with ASCII redraws marked with the PDF page (pp. 19, 20, 21).
7. **Clause 3 glossary:** term name and definition run together in the text layer; split from the render (bold term lines), numbering and the
   two alternative terms (3.1.26 "Media Presentation Description / MPD", 3.1.48 "stream access point / SAP") kept. The 3.2 abbreviation table
   spanned three pages as three tables with the first row taken as header; merged into one `Abbreviation | Term` table.
8. **XML syntax blocks** flattened to one line (and the 5.2.2 schema block turned into a pseudo-table). Re-broken into lines; whitespace and indentation
   are normalised, tag/attribute content is as printed (checked against the render of every block).
9. **Paragraphs split across lines/pages** ("... in the" / "MPD element.", "... Asset" / "Identifiers.", "... the" / "Supplemental descriptor.") re-joined
   (26 merges, each checked on the render).
10. **Page-footer rows** that the converter had put inside tables ("| 18 | (c) ISO 2022 ... |") removed; page boundaries are kept as `<!--PB printed N-->` markers.
11. **Markup characters:** straight/curly quote forms follow the text layer; U+2032 PRIME used for apostrophes in `'static'`/`'dynamic'` was normalised to `'`
    (render is indistinguishable) except in "Period′s", which is kept.

### Formatting policy

Words and punctuation are verbatim. Typography is reproduced only where cheap and unambiguous: bold element names, `@attributes` as code, subscripts,
bold/italic defined-term emphasis in running text, bold clause-3 terms. Not reproduced: italics on cross-referenced terms in clause 3, font changes
inside code-font identifiers, exact table borders/column widths, exact indentation of XML and of nested list levels in the PDF.

## 4. Consolidation with existing files

| Existing file | Disagreement with the fresh verification? | Action |
|---|---|---|
| `dash-23009-1-2022-aspect.md` (was `transmux/docs/codec/`) | none: `@par` (Table 5, printed p. 42), `@width`/`@height`/`@sar` (Table 14, printed p. 64) rows read identically on the render | moved with `git mv`; duplicated blockquotes replaced by links to Tables 5/14 here; analysis kept |
| `dash-23009-1-2022-ids.md` (was `transmux/docs/codec/`) | none: `AdaptationSet@id`/`@group` (Table 5, p. 41), `Representation@id` (Table 9, p. 53) identical | moved with `git mv`; duplicated blockquotes replaced by links; xsd-type table and analysis kept |
| `specs/rules/dash-mpd-rules.md` (2012 first edition) | **2 rules contradicted** (the `sidx` "shall be present" binding; `S@t` described as Period-relative) and several rules missing; see the check section at the end of that file | marked superseded, rule-by-rule list appended |
| `mp4-emsg/docs/emsg.md`, `specs/rules/emsg-rules.md` | not applicable (clause 5.10 is outside the range; only the `emsg` brand row of Table 1 touches it) | linked from INDEX, nothing duplicated |

Every reference to the old `transmux/docs/codec/dash-23009-1-2022-*.md` paths was updated (`docs/display-geometry/DESIGN.md`, `transmux/CHANGELOG.md`,
`transmux/src/dash.rs`, `transmux/docs/codec/dashif-iop-4.3-trickmode.md`).

## 5. Where two sources disagree

- Ed.5 print vs the Ed.6 schema files: [`ED6-SCHEMA-DIFF.md`](ED6-SCHEMA-DIFF.md) (10 of 39 transcribed types differ). Ed.6 semantics were not adopted.
- Within Ed.5 itself (printed as-is, flagged as spec notes): wrong cross-references (3.1.13/3.1.30 -> "3.1.50", 3.1.34 -> "3.1.24", 5.1 -> "5.2.3.5",
  5.3.9.4.4 -> "Table 20"/"Table 16", Table 22 `@k` -> "5.3.9.6.3"), broken cross-reference fields printed as "subclause 0", `(k-1) + (kstart-1)` vs
  `(k-1) + NumberStart` in 5.3.9.5.3, typos ("self-initialitizing", "sublause", "Represention", "Seqment", "seqment").

## 6. GAPS (what these sources do not cover)

- **Clause 7.2.1 / Annex A derivations.** The task asked for the normative derivation formulas for `MPD@availabilityStartTime`,
  `@timeShiftBufferDepth`, `@minimumUpdatePeriod`, `@suggestedPresentationDelay`, `@minBufferTime`, `@maxSegmentDuration`,
  `@maxSubsegmentDuration`, `@mediaPresentationDuration`, `@publishTime`, `@type`. In clause 5.3.1.2 (Table 3) these have *semantics only*; the
  timing model that uses them is in clause 7.2.1 (Media Presentation timeline) and Annex A (informative client behaviour), both outside the clauses
  assigned to this transcription. Within the range, the only availability formulas are in 5.3.9.5.3 (Segment availability start/end time,
  `@availabilityTimeOffset`), 5.3.9.5.2 (Initialization Segment availability) and 5.3.9.6 (SegmentTimeline), all transcribed.
- Clause 5.3.10 onward (Label/GroupLabel, Preselection, Initialization Set, Resynchronization), 5.4-5.15, clauses 6-8, Annexes A-K: not transcribed.
- Annex B (the normative Ed.5 schema) is not transcribed; only the clause-5 syntax snippets are. Annex G examples are not transcribed
  (Ed.6 example files exist under the vendored schema directory but were not compared).
- Figures 1-3 are redraws, not reproductions; the exact graphics are only on the PDF pages named in the redraw captions.
- Ed.6 prose (paywalled) not read.
- Cross-references printed as "subclause 0" are broken in the source; their intended targets are not guessed here.
- `emsg`/event clauses (5.10) are not part of this transcription (see `mp4-emsg/docs/emsg.md`).
