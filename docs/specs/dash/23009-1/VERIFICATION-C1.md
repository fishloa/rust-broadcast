# Verification log - branch C1 (public summary)

Full per-page defect log with exact fixes is in the private tree: `private/specs-verified/dash/23009-1/VERIFICATION-C1.md`. This public file
records only process, exit codes, and defect classes (no spec text).

## Source and tool

- PDF: `private/specs/iso_iec_23009-1_dash_2022_ed5.pdf` (330 pages). PDF page = printed page + 8.
- Converter: `cd ~/Projects/pdf2md && uv run pdf2md convert <pdf> -o <out.md> --pages A-B --engine hybrid --report` (model prefetch done first
  with `uv run pdf2md-prefetch`).

## Conversion runs (all engine `hybrid`, `--report`)

| Output file (private) | PDF pages | Printed pages | Exit code | Report output |
|---|---|---|---|---|
| `08-profiles.md` | 186-217 | 178-209 | 0 | empty (no flagged tokens) |
| `annex-a-client-behaviour.md` | 218-241 | 210-233 | 0 | empty |
| `annex-b-schema.md` | 242 | 234 | 0 | empty |
| `annex-c-mime.md` | 243-249 | 235-241 | 0 | empty |
| `annex-d-metrics.md` | 250-258 | 242-250 | 0 | empty |
| `annex-e-byte-range.md` | 259-261 | 251-253 | 0 | empty |
| `annex-f-extending.md` | 262-263 | 254-255 | 0 | empty |
| `annex-g-examples.md` | 264-291 | 256-283 | 0 | empty |
| `annex-h-srd.md` | 292-300 | 284-292 | 0 | empty |
| `annex-i-urlparam.md` | 301-314 | 293-306 | 0 | empty |
| `annex-j-open-gop.md` | 315 | 307 | 0 | empty |
| `annex-k-service-description.md` | 316-326 | 308-318 | 0 | empty |

Exit code 0 means every numeric token in the conversion matched the PDF text layer. It does not prove table structure, reading order, figures
or superscripts; those were checked against rendered pages for the areas below.

## Pages rendered (`pdftoppm -r 110`) and compared

PDF 203 (Figure 7), 220 (Table A.1), 223 (A.4 PTS formula), 230 (Figure A.1), 233-236 (A.13.4-A.13.7 formulas and figures), 240 (Table A.2),
245-246 (Table C.1), 260-261 (Table E.1, E.3), 275 (G.9/G.10 XML), 303-304, 311-312 (Tables I.1-I.4), 318-319, 322-323 (Tables K.1, K.2, K.5-K.8).
Not every page of the 141 converted was individually rendered; see gaps.

## Defect classes found in the text-layer conversion (all in table/figure/formula structure)

| Where | Class | Resolution |
|---|---|---|
| Tables A.1, A.2; C.1; E.1 and E.3 examples; I.1-I.4; K.1-K.9 | cells detached from their rows or interleaved | rebuilt in the private copy from the render |
| A.13.4, A.13.5 formulas, decode() | formula scrambled and prose merged | rewritten in the private copy; printed quirks recorded (`PS` vs `PST`; illegible v0 numerator) |
| A.4 `PTSs % 2^33` | superscripts lost | corrected in the private copy from the render |
| Figures A.1-A.6, 7 | text-layer shreds | replaced by a bracketed description in the private copy; redrawn in the public notes |
| Annex G XML blocks | line structure collapsed onto one line | left as converted; the Ed.6 `example_*.mpd` files carry line-structured text |

## Public notes: how each claim was grounded

Every public notes file cites printed pages and names its private verbatim file. Where the printed text looks inconsistent (examples
below) the public file carries a `> **Spec note:**` block rather than correcting it: 8.8.2.3 `@startsWithSAP` spelling; 8.12.4.5 `@t + (@r-1)*@d`;
A.3.3 `r[s]` loop count and undefined `d[0]`; A.13.4 formula symbols; I.3 `@headerParamSource` default (table vs text vs syntax); K.3.5
`TargetBandwidth` wording; K.5 table `OperatingBandwidth` description.

## Gaps (honest)

- Not every converted page was rendered; pages not in the list above were verified only by the exit-0 numeric check plus reading the converted text.
- Annex D and Annex H were converted (private) but not given structured public notes; Annex H H.3 examples and Annex D tables D.1-D.6 are unread by us beyond headings.
- The Ed.5 PDF does not embed the XSD (Annex B is only a URL). The Ed.5-exact `DASH-MPD.xsd` / `DASH-MPD-UP.xsd` was not fetched; the Ed.6 files in
  `private/specs/iso_iec_23009-1_dash_2026_ed6_schema/` were used for validation of the fresh examples.
- `DASH-MPD-PATCH.xsd` includes a network schema (`patch-ops.xsd` at iana.org); the patch example F8 was not schema-validated.
- Clauses 5.3.10 onward, 5.4-5.16, 6 and 7 are not transcribed in either branch; several clause-8 and Annex A statements refer to them.
- The Ed.6 XSD rejects `Event@messageData` (attribute) which Ed.5 text and G.9 use; unresolved here (see the comment in F4).
- Figure content (Figures 7, A.1-A.6, K.1) is described in prose or as a rough redraw; none carries extra normative detail beyond the text.
