# st2022 — spec prep (issue #752)

Prep-only docs for a future SMPTE ST 2022-6/-7 crate (SDI-over-IP transport +
seamless protection switching). **No Rust source or `Cargo.toml` exists yet**
— this directory is the spec-grounding step that has to exist before
implementation is delegated, per the project's "Spec grounding" convention
(root `CLAUDE.md`).

## Sources

| Document | Obtained from | Date fetched | Local scratch copy |
|---|---|---|---|
| SMPTE ST 2022-6:2012, "Transport of High Bit Rate Media Signals over IP Networks (HBRMT)" | `https://pub.smpte.org/pub/st2022-6/st2022-6-2012.pdf` (free, public SMPTE catalog) | 2026-07-30 | not committed — re-fetch from the URL above if needed |
| SMPTE ST 2022-7:2019, "Seamless Protection Switching of RTP Datagrams" (current edition) | `https://pub.smpte.org/latest/st2022-7/st2022-7-2019.pdf` (free, public SMPTE catalog) | 2026-07-30 | not committed |
| SMPTE ST 2022-7:2013, "Seamless Protection Switching of SMPTE ST 2022 IP Datagrams" (predecessor edition, consulted only for wording that 2019 dropped/softened/generalized) | `https://pub.smpte.org/doc/st2022-7/20131011-pub/st2022-7-2013.pdf` (free, public SMPTE catalog) | 2026-07-30 | not committed |

The **private submodule** (`private/specs/`, repo `rust-broadcast-private`)
was checked first per the workspace convention — `git submodule update --init
private` was run and the submodule is present, but it does **not** carry any
SMPTE ST 2022-6 or ST 2022-7 PDF (its SMPTE holdings are RDD 29, ST 12-1,
ST 337, ST 377-1, and ST 2038 only, none of which are ST 2022-6/-7). All
three PDFs above were instead obtained directly from the free public SMPTE
catalog at `pub.smpte.org`, per the task brief's pointer. None of these three
PDFs have been added to `private/` or committed anywhere in this repo by
this task — they are scratch copies only, and this README records the exact
URLs so anyone can re-fetch them.

All three PDFs were converted to Markdown locally with the repo's `pdf2md`
tool (`--engine textlayer`, text-layer-based, no cloud OCR), which
cross-checks every `0x…`/digit-run token in its output against the PDF's own
embedded text layer and reports a diagnostic (and non-zero exit) for any
mismatch:

- **ST 2022-6:2012**: exit code 1, 18 diagnostics, **all on page 12** (the
  **informative** Figure 5 worked TRS-bit-packing example). Every flagged
  token's reported "engine" value and "text-layer" value are identical
  (e.g. `0x3FF`/`0x3FF`, `0xC0`/`0xC0`) — the tool's diff logic flagged a
  position/ordering artifact in that one informative table, not a value
  corruption. Nothing in the normative header field tables (§6.3/§6.4/§6.5)
  triggered a diagnostic. Cross-checked independently against a plain
  `pdftotext -layout` extraction of the same PDF — the two extraction
  methods agree byte-for-byte on every field name, bit width, and coded
  value used in [`st2022-6-framing.md`](st2022-6-framing.md).
- **ST 2022-7:2019**: exit code 0, no diagnostics.
- **ST 2022-7:2013**: consulted via `pdftotext -layout` only (used for
  cross-version comparison text, not as the primary transcription source —
  2019 is authoritative).

## What each doc covers

- [`st2022-6-framing.md`](st2022-6-framing.md) — RTP/UDP/IP header (§6.3),
  the full HBRMT Payload Header field-by-field (§6.4: fixed row + Video
  Source Format row + Video Timestamp + TLV Header Extension), the Media
  Payload structure and last-payload sizing equations (§6.5), a frame/field
  structure summary, and the FEC interoperability *limits* (§7.1 — not the
  FEC wire format itself, which lives in a different standard, see below).
- [`st2022-7-hitless.md`](st2022-7-hitless.md) — the redundancy model (§6),
  the exact duplicate-identification rule a receiver needs (§4.3 + Annex A —
  this is the section written to directly answer "what does
  `media_plane::byte_merge::MergePolicy::Hitless2022_7` need to parse"), the
  Receiver Classification / skew table (§7 Table 1), and the informative
  buffer-sizing worked example (Annex A).

## Fidelity audit

These documents were audited adversarially against freshly downloaded copies of
both PDFs (2026-07-29) — every field width, offset, enum coding table, sizing
equation and clause number re-checked independently, because a transcription is
not an oracle until someone has tried to break it. Findings recorded in
`.delegate/752-fidelity-audit.md`.

**One severe defect was found and is now corrected:** the ST 2022-7 redundancy
section claimed that 2013 required *exactly* two streams and that 2019 relaxed
this to "at least two". Both editions in fact say "at least two" identically;
the only two-vs-more change is in the Definitions clause (2013 §5.10 "two" →
2019 §4.10 "two or more"). The claim was stated unhedged, so it would have
misled an implementer about a normative requirement that never changed — and it
was the one thing this gaps list should have contained and did not, because the
transcriber did not know it was wrong. That is precisely why the audit is a
gate and not an optional extra.

Everything else verified accurate, including both items flagged below.

## Could NOT establish (explicit gaps — do not fill from memory)

1. **HBRMT Payload Header `RESERVE` field's exact bit width (5 bits, in
   `st2022-6-framing.md` §3.1).** The spec's running prose never states a
   bit count for this field (every neighboring field does). The 5-bit value
   is arithmetically derived from the explicitly-stated widths of the other
   four fields in that 16-bit half-row (`16 − 2 − 2 − 3 − 4 = 5`), which is
   forced by the diagram's 32-bit row boundary — not an invented number, but
   also not a directly-quoted one. Flagged inline in the doc; worth a second
   look before an implementation bakes in the exact bit offset.
2. **SMPTE ST 2022-5** (the separate FEC wire-format standard that ST 2022-6
   §7.1 references for the actual FEC Header/FEC Payload byte layout) —
   **not obtained**. ST 2022-6 only specifies interoperability *limits* on
   the `L`/`D` matrix parameters (transcribed in `st2022-6-framing.md` §6);
   the FEC datagram's actual field-by-field format is not transcribed
   anywhere in this prep, because the source document was never fetched.
3. **SMPTE ST 2022-1/-2/-3/-4** (referenced by ST 2022-7 as example "Class
   SBR" input-stream specs) — not obtained; referenced by name only.
4. **The embedded-audio sub-structure inside the HBRMT Media Payload.**
   ST 2022-6 states that "all TRS, VANC, and HANC" (i.e. the entire raw SDI
   signal, including embedded audio) is carried as one opaque byte stream,
   and defines no audio-specific mapping of its own. The actual embedded-audio
   format (e.g. SMPTE ST 272/299 or equivalent) was not fetched or
   transcribed — it is out of ST 2022-6's own scope, so this is flagged for
   completeness rather than treated as a real gap in the HBRMT framing spec.
5. **The 2^32/27MHz RTP-timestamp-rollover arithmetic discrepancy between
   editions** (`st2022-7-hitless.md` §6): the 2019 edition states "≈159.07
   seconds (2^32/27,000,000)", which checks out arithmetically
   (4,294,967,296 / 27,000,000 ≈ 159.07). The 2013 edition states "40,722.6
   seconds (2^32/27M)" for the same quantity, which does **not** check out
   against that formula as written. Both numbers are quoted verbatim from
   their respective PDFs; this prep does not attempt to resolve which (if
   either) is a typo in the original SMPTE document, since doing so would
   require guessing SMPTE's intent rather than reading it from a source.
6. **Whether a newer errata/amendment of ST 2022-7:2019 exists** beyond what
   `pub.smpte.org/latest/...` served. The "latest" URL path suggests this is
   the current edition, but no exhaustive search for a subsequent revision
   was performed beyond the web search that surfaced these three PDFs.
7. **The precise selection/arbitration *algorithm*** a Hitless2022_7
   producer should use (e.g. strict first-arrival-wins vs.
   prefer-primary-VSID-unless-late) is **not** specified by SMPTE — §7 of
   ST 2022-7:2019 explicitly states "the exact method of reconstruction is
   left to the implementer." This is not a gap in the transcription; it is
   the standard genuinely leaving that choice open. Flagged here so it isn't
   mistaken for missing research.

## Style note

Layout follows the existing `st291/docs/` house style (flat
one-file-per-topic under `docs/`, source citation + verification note at the
top of each file, per-field tables citing clause numbers) rather than
`dvb-si/docs/`'s deeper `tables/`/`descriptors/`/`enums/` split, since ST 2022-6/-7
— like ST 291-1 — is a small, single-spec-family crate rather than a
multi-table SI standard.
