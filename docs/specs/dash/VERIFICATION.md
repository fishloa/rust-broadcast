# Verification summary - DASH client spec notes (IOP v4.3, IOP Part 10 v5.0.0, ETSI TS 103 285 v1.4.1)

The public notes in this tree are original-wording; the full per-page verification records, including every
text-layer-vs-render mismatch and how it was resolved, live with the private verbatim transcriptions:

- `private/specs-verified/dash/dashif-iop-4.3/VERIFICATION.md`
- `private/specs-verified/dash/dashif-iop-part10-v5.0.0/VERIFICATION.md`
- `private/specs-verified/dash/etsi-ts-103-285/VERIFICATION.md`

(private submodule branch `dash-specs-C2`, not pushed; the public submodule pointer is intentionally not bumped).

## Method (all three documents)

1. Text layer (pdftotext/pypdfium2) used only as a typing aid; `pdftotext -layout` only to navigate.
2. Every transcribed page rendered with `pdftoppm -r 110` and read; every table, XML/ABNF block, nested list and
   figure rebuilt from the rendered page; selected pages re-rendered at 150-220 dpi.
3. pdf2md hybrid run per range with `--report`, exit code recorded; exit 0 means every numeric/hex token matched the
   text layer.
4. Automated token cross-checks (digit/URN tokens, word shingles) between the PDF text layer and the transcription.
5. Public notes were then written from the verified text; spot facts re-checked; public notes were screened against
   the private verbatim with a 10-word shingle overlap check (longest shared run in any note: 49 words, an
   Annex D identifier/resolution table; all others <= 29, mostly URN/metadata lists).

## pdf2md runs

| Document | Pages | Engine | Exit code |
|---|---|---|---|
| IOP v4.3 | PDF 3-82 | hybrid | 0 |
| IOP v4.3 | PDF 82-143 | hybrid | 0 |
| IOP v4.3 | PDF 143-254 (text layer) and 144-254; table pages 165-168, 173-174, 178, 182-184, 200-205, 212, 218, 221, 223-224, 241-243, 253 (hybrid) | textlayer / hybrid | 0 / 0 / 0 |
| IOP Part 10 v5.0.0 | PDF 1-19 | hybrid | 0 |
| TS 103 285 | PDF 9-61 | hybrid | 0 |
| TS 103 285 | PDF 61-149 | hybrid | 0 |

## Mismatch classes found (details in the private records)

Tables whose cells collapse, split or interleave in the text layer (IOP Tables 1, 2, 4, 7, 8, 11, 13, 15, 16, 19,
timing-example tables; TS Tables 1-4, 6, 8-10, 13-34); line-number margins in the IOP PDF becoming table cells in
every existing conversion; soft-hyphen losses in URNs and codec strings; nested lists flattened; XML samples run
into prose; figures without text layer (IOP Figures 1-10; TS BaseURL example boxes, Figure 6). All were resolved
from the rendered pages.

## Source defects recorded (with page cites) in the notes

Examples: IOP `isoff-ondemand` vs `isoff-on-demand`; Role `alternative` vs `alternate`; period-continuity `@value`
Period id vs Adaptation Set id; `urn:mpeg:dash:event:2014` vs `2012`; Table 15 `$Time$` vs "`$Number$` only";
`event_duration` unknown printed `0xFFFF`; `hvc1.12.L123.B0`; thumbnail 256/180 sizes vs formulas; TS 103 285
5.3.2.5 title/body mismatch, DTS-UHD `tag` values, AVS3 MatrixCoefficients 8 vs 9, wrong clause cross-references,
10.20.3 example arithmetic; Part 10 §6.1 "may be ignored" wording and the `InbandEventStream@presentationTimeOffset`
disagreement with `specs/rules/emsg-rules.md`.

## Gaps

- IOP Annexes C, E, F not transcribed (ISOBMFF/SEI payload syntax, not client-visible); TS clauses 11.2-11.3 tables
  (PDF 114-117) and the document history not transcribed.
- Not in the repository: SCTE 214-1, the TS 103 285 DVB extension XSD, TS 101 154, RFC 7231/7232/7164 text,
  Amendment 3 of the 2014 23009-1 edition (the `posix:` MPD anchor wording comes from the IOP alone).
- Cited 23009-1 clauses (IOP uses 2nd-edition-era numbering; TS 103 285 cites the 2020 edition) were not compared with
  the 2022 transcriptions in `transmux/docs/codec/`.
- The F3 hybrid conversion output and the TS F4 raw output were not retained.
- The client reference model and API promised by Part 10 §1/§4 do not exist in the published text.
