# ttml-subtitle spec docs — provenance and gaps

Prep work for GitHub issue #753 ("new crate: TTML / IMSC subtitles — W3C
TTML2 + IMSC 1.1"). This directory contains **transcribed spec reference**
only — no Rust source, no `Cargo.toml`. Real fixtures live in
`../tests/fixtures/` (see its own `PROVENANCE.md`).

## Sources fetched

| File | Spec | Exact version fetched | URL | Fetched |
|---|---|---|---|---|
| `ttml2-syntax.md` | W3C TTML2 | Recommendation, 08 November 2018 | `https://www.w3.org/TR/2018/REC-ttml2-20181108/` (latest alias: `https://www.w3.org/TR/ttml2/`) | 2026-07-30 |
| `imsc11-profiles.md` | W3C IMSC 1.1 | Recommendation, 08 November 2018, edited in place 27 April 2020 | `https://www.w3.org/TR/2018/REC-ttml-imsc1.1-20181108/` (latest alias: `https://www.w3.org/TR/ttml-imsc1.1/`) | 2026-07-30 |

Method: the published HTML of each Recommendation was fetched directly
(`curl`) and its own `<table class="syntax">` / `<table class="common">` /
feature-disposition `<table>` markup was parsed and transcribed verbatim
(cell-for-cell, respecting `colspan`), rather than relying on prose
paraphrase or an LLM summary of the page — this was a deliberate choice to
eliminate the risk of a fabricated section number or attribute value. Prose
sections that carry normative constraints (timing semantics, common
provisions §7, profile constraints §8/§9) were extracted as heading-scoped
plain text and read/transcribed by hand from the actual spec wording, not
from memory of TTML1/earlier IMSC versions.

Both documents are still current — no newer TTML2 or IMSC revision has
superseded either REC as of the fetch date (checked via each page's own
"Latest published version" link, which resolved to the same document).

## What could NOT be established from a readable source (do not guess these)

The following are referenced by the specs but were **not** transcribed
because doing so would have required either (a) content not practical to
verify from the fetched HTML (image/table-heavy appendices), or (b) material
outside what this prep pass had time to fully read. Anyone implementing
against these tables must go back to the primary spec text — do not infer
values from the summaries in `ttml2-syntax.md`/`imsc11-profiles.md`:

- **TTML2 §10.4 Style Resolution algorithm** (the full CSS-cascade-like
  resolution procedure: specified → computed → used value, per-property
  categories) — only cited by section number, not transcribed. Needed for a
  *presentation* processor; not needed for parse/serialize.
- **TTML2 §11.3 Rendering Model / line layout** (glyph-area/line-area
  generation, synchronic flow processing) — cited by reference only.
- **TTML2 Appendix H Root Container Region Semantics** — the aspect-ratio
  derivation algorithm when `ttp:displayAspectRatio`/`ttp:pixelAspectRatio`
  are absent is referenced by name throughout `ttml2-syntax.md` but its
  actual case-by-case derivation rules (Appendix H.1–H.3) were not
  transcribed.
- **TTML2 Appendix I Time Expression Semantics**, beyond the summary in
  `ttml2-syntax.md` §3.7 — the full clock/media/smpte time-base
  interpretation semantics (I.1–I.3) were not transcribed in detail.
- **TTML2 Appendix C schemas (RNC/XSD)** — not transcribed; the syntax boxes
  in §3 of `ttml2-syntax.md` are the spec's own human-readable equivalent and
  were judged sufficient, but a from-scratch XSD-driven validator would need
  the actual schema text, not reproduced here.
- **IMSC 1.1 Annex A Reference Fonts** — the actual font-name/metrics table
  (which fonts satisfy §8.3's "reference font" requirement for which
  script/codepoint combinations) was not transcribed. This table is
  presentation-processor-relevant (glyph metrics), not parse/serialize, so
  it was deliberately deprioritized, but it is a real content gap if this
  crate's scope ever extends to rendering.
- **IMSC 1.1 Annex B Common Character Sets** — the recommended-input
  character ranges per §8.2 were not enumerated.
- **IMSC 1.1 Annex I Compatibility with other TTML-based specifications**
  (EBU-TT-D, SDP-US, SMPTE-TT, CFF-TT, IMSC1/1.0.1) — read only for the
  Text/Image-Profile-override designators reused in `imsc11-profiles.md`
  §2; the full compatibility discussion for each sibling spec was not
  transcribed. If this crate needs to interoperate specifically with
  EBU-TT-D documents (a real broadcast concern — EBU-TT-D is IMSC-adjacent
  and common in European subtitle workflows), that annex should be read in
  full and probably given its own doc file before implementation.
- **IMSC 1.1 Annex L Summary of substantive changes** (1.0.1 → 1.1 diff) —
  not transcribed; `imsc11-profiles.md` notes only that the two designator
  families are distinct, not what changed between them feature-by-feature.
- **TTML2's own `ttml2-syntax.md` §10.2.1 (`style` binding attribute) and
  the non-normative appendices** (K–V: QA framework, security/privacy, HDR
  compositing §Q, streaming fragmentation §R, styling examples §S,
  presentation-customisation §T, TTML1 diff §U) — deliberately out of scope
  for a container/carriage crate; listed in `ttml2-syntax.md` §6 for
  visibility.
- **Whether any TTML2/IMSC erratum has been published against either 2018
  REC since 27 April 2020** — checked only via each spec's own "latest
  version" self-link (both resolve to the fetched document); no separate
  errata-tracker page was checked. If accuracy against a post-2020 erratum
  ever matters, re-verify against `https://www.w3.org/TR/ttml2/` and
  `https://www.w3.org/TR/ttml-imsc1.1/` directly before relying on this
  transcription.

Nothing above was filled in from memory/recollection of TTML1, DFXP, or
other subtitle formats — where the fetched spec text did not make a value
explicit and reachable within this pass, it is listed above rather than
guessed.
