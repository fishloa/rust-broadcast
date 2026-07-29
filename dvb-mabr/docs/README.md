# dvb-mabr — spec table reference

Prep work for issue **#755** ("new crate: DVB multicast ABR (DVB-MABR, ETSI TS 103
769)"). This directory is **docs only** — no `Cargo.toml`, no Rust source. It exists so
that implementation (delegated or otherwise) starts from a reviewable, spec-cited
transcription rather than from memory of the spec.

## Source

- **Standard**: ETSI TS 103 769, "Digital Video Broadcasting (DVB); Adaptive media
  streaming over IP multicast".
- **Edition read for this transcription**: **V1.2.1 (2024-11-12)** — the current
  edition. An older **V1.1.1 (2020-11-09)** edition also exists on the ETSI server but
  was **not** read; V1.2.1 supersedes it and is what a new implementation should target.
  If a future task needs to diff behaviour against V1.1.1, that edition would need a
  separate transcription pass.
- **Where it was obtained**: `private/specs/` (this workspace's private submodule, the
  usual first stop for vendored spec PDFs) was checked first via `git submodule update
  --init private` and does **not** contain a DVB-MABR PDF (confirmed by listing
  `private/specs/`: no `103769`/`mabr` file present, and `grep`/`find` across the whole
  workspace found nothing). The PDF was therefore fetched directly from the public ETSI
  deliverables server:
  `https://www.etsi.org/deliver/etsi_ts/103700_103799/103769/01.02.01_60/ts_103769v010201p.pdf`
  (fetched 2026-07-30; HTTP 200, 1 649 894 bytes, PDF 1.7). ETSI publishes this
  deliverable free of charge for download but retains copyright over redistribution
  ("No part may be reproduced or utilized in any form ... except as authorized by
  written permission of ETSI" — title page). Consistent with this workspace's existing
  split (freely-redistributable text specs live in the public `specs/` dir; copyrighted
  vendor PDFs live only in the private `rust-broadcast-private` submodule, never in the
  public tree), **the PDF itself was not committed here** — only the field-level
  transcription in this directory (the same treatment `dvb-si/docs/` gives to its ETSI
  EN 300 468 PDF). **Follow-up for a maintainer with private-submodule write access**:
  add `ts_103769v010201p.pdf` to `private/specs/` so future audits don't need to
  re-fetch it, matching how every other ETSI PDF in this workspace is vendored.
- **How it was read**: converted to Markdown locally with the `pdf2md` skill
  (`--engine textlayer`, whole 185-page document, `--report`). Exit code **0** — every
  `0x`/`0b`/digit-run token in the conversion matched the PDF's embedded text layer, so
  no numeric/hex value was dropped or corrupted by the conversion for the parts actually
  used below. `pdftotext` was deliberately not used (per project convention — it
  scrambles table columns).
  - One caveat found *by inspection*, not by the verifier: multi-page tables that
    reflow columns across a page break (the reference-architecture box diagrams in
    clauses 5-7, and the JSON reporting-schema table, clause 11.1.1) come out with their
    cells in the wrong order even though every individual token is byte-correct — the
    verifier only checks token fidelity, not column/row order. Those two areas are
    called out explicitly below and were **not** transcribed from the garbled table
    text; only the surrounding prose (which reads as ordinary paragraphs and was not
    affected) was used.

## Files in this directory

| File | Covers |
|---|---|
| [`mabr-architecture.md`](mabr-architecture.md) | Reference architecture: logical functions, reference points (clause 5), deployment models (clause 6), modes of system operation incl. rendezvous request/redirect syntax (clause 7). |
| [`mabr-transport.md`](mabr-transport.md) | Multicast transport object formats: the FLUTE profile (Annex F), the ROUTE profile (Annex H, excluding the ATSC A/331 packet-header bytes themselves — see below), FEC (clause 8.3.4.2), unicast repair (clause 9), and the integrity/authenticity metadata profiles (clause 12) both annexes build on. |
| [`mabr-signalling.md`](mabr-signalling.md) | The multicast session configuration XML document: data model (clause 10), extensibility mechanism and full baseline schema (Annex A), classification schemes (Annex B), and a pointer at the JSON reporting document (clause 11, not fully transcribed — see below). |

**33 distinct wire/data structures** are documented with a field table across the three
files: the rendezvous request/redirect URL syntax (2 tables, clause 7), 3 root/session
XML elements (`MulticastServerConfiguration`, `MulticastGatewayConfiguration`,
`MulticastSession`), the `MulticastTransportSession` element (the largest, with 4 nested
sub-structures: `EndpointAddress`, `ForwardErrorCorrectionParameters`,
`UnicastRepairParameters`, `ObjectCarousel`), `MulticastGatewayConfigurationTransportSession`,
`PresentationManifestLocator`, 3 `ServiceComponentIdentifier` variants (DASH/HLS/generic),
`MulticastGatewaySessionReporting`, 3 classification-scheme vocabularies (Annex B), the
FLUTE Extended-FDT `Signature`/digest attributes (Annex F.2.3-F.2.4), the ROUTE
`Table H.2.0-1` LCT-field constraints and the `S-TSID` mapping tables (H.5.0-1/2/3), and
the two HTTP security profiles (chunk-digest / chunk-signature ABNF, clause 12).

## Overlap with existing crates and issues in this workspace

- **`dvb-flute`** (published, v0.3.0) already implements the byte-level LCT header (RFC
  5651), ALC packet + `EXT_FTI` (RFC 5775), and FLUTE `EXT_FDT`/`EXT_CENC` (RFC 6726) —
  exactly the packet layer TS 103 769 Annex F profiles. **`dvb-mabr` should depend on
  `dvb-flute`, not duplicate it.** See `mabr-transport.md` §0/§6 for the detailed
  breakdown of what Annex F adds on top (an XML extension to the FDT body, a
  chunked-mode URI convention) versus what it inherits unchanged from `dvb-flute`.
- **Issue #750** ("new crate: ATSC 3.0 (A/331 ROUTE + MMT signalling, A/321
  bootstrap)") is open and unimplemented. TS 103 769 Annex H (the ROUTE-based transport
  profile) is built entirely on top of ATSC A/331 Annex A's ROUTE/LCT packet format,
  which is squarely #750's scope (`EXT_TOL`, `EXT_TIME`, the ROUTE Codepoint registry,
  S-TSID parsing). **`dvb-mabr`'s Annex H support cannot be fully implemented until
  #750 lands** (or `dvb-mabr` builds a minimal, possibly-throwaway ROUTE primitive set
  of its own — a design decision for whoever picks this up, not made here). Annex F
  (FLUTE) has no such blocker.
- **`transmux`** already covers DASH/HLS/CMAF container muxing in this workspace; DVB-MABR
  references DASH (ISO/IEC 23009-1) and HLS (IETF RFC 8216) manifests but never redefines
  their syntax — `dvb-mabr` treats presentation manifests as opaque URLs/bytes it
  relays, never parses. No overlap requiring a design decision, but worth noting that
  manifest-content-aware behaviour described informatively in clause 8.4.1.1 (MPD
  `@timeShiftBufferDepth`/`Location`/`SegmentTemplate` manipulation) would naturally
  compose with `transmux`'s existing DASH support rather than reimplementing DASH MPD
  parsing inside `dvb-mabr`.

## Explicit list of what could NOT be established from a readable source

Per the project's "never invent a value" rule, these are named here rather than guessed:

1. **The exact bit layout of `EXT_TOL` and `EXT_TIME`** (ROUTE-specific LCT header
   extensions used by Annex H, clause H.2.1) — defined in ATSC A/331 Annex A, which is
   not vendored or transcribed anywhere in this workspace. Needed before Annex H can be
   implemented; see the #750 overlap above.
2. **The ROUTE `Codepoint` registry's full semantics** (ATSC A/331 Table A.3.6) — TS 103
   769 clause H.4 only says Codepoints 5-9 are used and what each means at a summary
   level (init segment vs. media segment, File vs. Entity mode); the full registry
   table lives in ATSC A/331, not read for this pass.
3. **The overall ROUTE packet diagram** (clause H.2.0, figure H.2.0-1: UDP header +
   Default LCT header + FEC Payload ID + payload) — present in the PDF as an ASCII-art
   bit-ruler diagram that the textlayer conversion reproduced as a garbled fragment
   sequence, not as a clean table; the *prose* around it (Table H.2.0-1, which is a real
   table and read cleanly) was used instead. The diagram itself adds no information
   beyond "this is the standard ALC/LCT/FEC-Payload-ID/payload stacking" already
   documented in `dvb-flute/docs/alc.md`.
4. **The full JSON service-reporting schema** (clause 11.1.1, Table 11.1.1-1, and the
   per-event-type property tables in clauses 11.1.2.2-11.1.2.5) — these are multi-page
   tables that reflowed into scrambled column order in the textlayer conversion (see
   "How it was read" above). The clause *prose* (readable, not table-mangled) was used
   to record the three top-level event types, the `object-delivery-status` enum values,
   and the named per-session counters in `mabr-signalling.md` §6, but the complete
   property list (types, `Use` cardinalities, and several counter names visible only in
   the scrambled table, e.g. the `cache-hit`/`cache-miss` sub-metrics beyond
   `object-delivery-status`) needs a **re-run with a different pdf2md engine
   (`--engine hybrid` or `--engine marker`) or the `blaze` OCR skill** on pages
   approximately 85-92 of the PDF before the reporting document can be implemented.
   This gap does not block the three files actually required by this task (they cover
   architecture/transport/signalling of the *session configuration*, not the reporting
   protocol), so it was left as a named gap rather than delegated a token budget it
   didn't need for issue #755's acceptance criteria — but a future "reporting" story
   should not skip straight to implementation without first closing this gap.
5. **The reference-architecture box diagrams** (figures 5.1.0-1, 5.2-1, 6.1-1, 6.2-1,
   6.3-1, 7.1-1, 7.2-1) — same textlayer-diagram limitation as #3. The named reference
   points and workflow steps were fully recovered from the surrounding prose (which is
   not diagram content and reads cleanly), so no information required by
   `mabr-architecture.md` is actually missing — this entry just documents *why* no ASCII
   reproduction of the diagrams themselves appears in that file.
6. **DVB-MABR V1.1.1 (2020) vs. V1.2.1 (2024) diff** — not established; only V1.2.1 was
   read (see "Source" above). If back-compatibility with V1.1.1-only deployments ever
   becomes a requirement, that edition needs its own read.

Everything else field/attribute/element-level in `mabr-architecture.md`,
`mabr-transport.md`, and `mabr-signalling.md` was read directly from a `pdf2md`
conversion that the tool's own byte-level verifier confirmed as token-exact against the
PDF's text layer (exit code 0), from clean (non-scrambled) prose or tables.
