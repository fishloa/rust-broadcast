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
| [`mabr-signalling.md`](mabr-signalling.md) | The multicast session configuration XML document: data model (clause 10), extensibility mechanism and full baseline schema (Annex A), classification schemes (Annex B). |
| [`mabr-reporting.md`](mabr-reporting.md) | Service reporting information document (JSON): the full OpenAPI 3.0.1 YAML schema from normative Annex N, cross-checked against clause 11 prose — including the seven event types, all per-session counters, and the flagged `object-delivery-status` enum conflict (§2.2.1). |

**34 distinct wire/data structures** are documented with a field table across the four
files: the rendezvous request/redirect URL syntax (2 tables, clause 7), 3 root/session
XML elements (`MulticastServerConfiguration`, `MulticastGatewayConfiguration`,
`MulticastSession`), the `MulticastTransportSession` element (the largest, with 4 nested
sub-structures: `EndpointAddress`, `ForwardErrorCorrectionParameters`,
`UnicastRepairParameters`, `ObjectCarousel`), `MulticastGatewayConfigurationTransportSession`,
`PresentationManifestLocator`, 3 `ServiceComponentIdentifier` variants (DASH/HLS/generic),
`MulticastGatewaySessionReporting`, 3 classification-scheme vocabularies (Annex B), the
FLUTE Extended-FDT `Signature`/digest attributes (Annex F.2.3-F.2.4), the ROUTE
`Table H.2.0-1` LCT-field constraints and the `S-TSID` mapping tables (H.5.0-1/2/3),
the two HTTP security profiles (chunk-digest / chunk-signature ABNF, clause 12), and
the full service reporting JSON/OpenAPI schema (Annex N in
[`mabr-reporting.md`](mabr-reporting.md)).

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

Per the project's "never invent a value" rule, these are named here rather than guessed.
This list has been updated after the **adversarial fidelity audit** (2026-07-30 — see
"Audit history" below) and now records only genuine unknowns, not resolved gaps:

1. **The exact bit layout of `EXT_TOL` and `EXT_TIME`** (ROUTE-specific LCT header
   extensions used by Annex H, clause H.2.1) — defined in ATSC A/331 Annex A, which is
   not vendored or transcribed anywhere in this workspace. Needed before Annex H can be
   fully implemented; see the #750 overlap above.
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
4. **The reference-architecture box diagrams** (figures 5.1.0-1, 5.2-1, 6.1-1, 6.2-1,
   6.3-1, 7.1-1, 7.2-1) — same textlayer-diagram limitation as #3. The named reference
   points and workflow steps were fully recovered from the surrounding prose (which is
   not diagram content and reads cleanly), so no information required by
   `mabr-architecture.md` is actually missing — this entry just documents *why* no ASCII
   reproduction of the diagrams themselves appears in that file.
5. **DVB-MABR V1.1.1 (2020) vs. V1.2.1 (2024) diff** — not established; only V1.2.1 was
   read (see "Source" above). If back-compatibility with V1.1.1-only deployments ever
   becomes a requirement, that edition needs its own read.

**Resolved gaps** (formerly on this list, now fixed):
- ~~The full JSON service-reporting schema~~ — now transcribed from Annex N in
  [`mabr-reporting.md`](mabr-reporting.md). Annex N (page 179, the complete OpenAPI
  3.0.1 YAML) was initially missed entirely by the first transcription pass.

## Flagged spec-internal conflicts (do not resolve by picking one silently)

These two inconsistencies within TS 103 769 V1.2.1 itself were found by the audit.
Neither list below is speculative — both sides were read directly from the rendered
PDF and verified against the source:

1. **`object-delivery-status` enum — clause 11.1.2.2 vs. Annex N:**
   - **Table 11.1.2.2-1** (normative body, page 89) specifies 10 values:
     `cache-hit-m`, `cache-hit-a`, `cache-hit-mr`, `cache-miss-expired`,
     `cache-miss-incomplete`, `cache-miss-filter`, `cache-miss-nodata-s`,
     `cache-miss-nodata-m`, `cache-miss-nodata-j`, `cache-miss-nodata-o`.
   - **Annex N (normative OpenAPI YAML, pages 180, 183)** specifies 9 values:
     `cache-hit-m`, `cache-hit-a`, `cache-miss-incomplete`, `cache-miss-filter`,
     `cache-miss-nodata-s`, `cache-miss-nodata-m`, `cache-miss-nodata-j`,
     `cache-miss-nodata-o`, `cache-miss-timeshift`.
     (The YAML's `MABR_ObjectDeliveryStatus` on page 183 additionally contains a
     duplicate `cache-miss-expired` entry — a clear typo.)
   - Clause 11.1.2.2 has `cache-hit-mr` and `cache-miss-expired`; Annex N has
     `cache-miss-timeshift`. The other 8 values overlap.
   - An implementation that picks one side will emit reports the other side rejects.
     Documented in full in [`mabr-reporting.md`](mabr-reporting.md) §flag.

2. **Signature key attribute — `@keyUri` vs `keyId`:**
   - **Prose Table F.2.4.1-1** (page 137) names it `@keyUri`, typed as a URI string.
   - **Annex F.2.5.1 XSD** (page 138, the normative wire-format authority) names it
     `keyId`, typed as `xs:hexBinary`.
   - These disagree on both *name* and *type*. Documented in
     [`mabr-transport.md`](mabr-transport.md) §2.4.

## Audit history

**2026-07-30 — adversarial fidelity audit** (against the rendered PDF, not the
pdf2md conversion):

Seven defects were found across the three initial transcription files, three severe:

| # | Severity | Finding | File affected |
|---|---|---|---|
| 1 | **Severe** | Annex N (normative OpenAPI reporting schema, page 179) was missed entirely; transcription pointed at the garbled clause 11.1.1 table instead. Annex N is now transcribed in `mabr-reporting.md`, and the spec-internal `object-delivery-status` enum conflict between clause 11.1.2.2 (10 values) and Annex N (9 values) is flagged. | New: `mabr-reporting.md` |
| 2 | **Severe** | Table A.0-1 (schema version → required namespaces) was reversed — v1 listed as 2019+Extensibility, v2 as 2024. The table actually says v1 = 2019 only, v2 requires **two** namespaces: `Extensibility:2024` (Clause A.1) and `MulticastSessionConfiguration:2024` (Clause A.2). Fixed in `mabr-signalling.md` §9. | `mabr-signalling.md` |
| 3 | **Severe** | The Annex F.2.5.1 XSD was dismissed as "adds no information" but it names the key attribute `keyId` (typed `xs:hexBinary`), not `@keyUri` (URI string) as the prose table says — a second spec-internal conflict. Flagged in `mabr-transport.md` §2.4. | `mabr-transport.md` |
| 4 | Moderate | `MulticastGatewayConfigurationTransportSession`'s `PresentationManifests`/`InitSegments` cardinality silently changes `0..1` → `0..n` relative to the base element. Noted explicitly. | `mabr-signalling.md` |
| 5 | Moderate | The classification-scheme table fabricated `MSync/RTP` as a flat term; `RTP` is actually nested as a child `<Term>` under `MSync` in the XML schema. Fixed with term-path notation. | `mabr-signalling.md` |
| 6 | Minor | `@protocolVersion` typed as "string/positive integer"; the XSD says `xs:positiveInteger`. Corrected. ⚠️ This is a **prose-vs-XSD conflict**: the prose Table 10.2.3.1-1 says "String" but the XSD `MulticastTransportProtocolType` (Annex A.2) says `xs:positiveInteger` — same class as finding 3 above. Flagged in `mabr-signalling.md` §4. | `mabr-signalling.md` |
| 7 | Minor | `BitRate` description said "FEC included" unconditionally; clause 10.2.3.10 includes FEC only when repair packets are addressed to the **same** destination group network address. Conditional restored. | `mabr-signalling.md` |

**Meta-finding:** the initial gaps list was incomplete in exactly the dangerous way:
findings 2 and 3 were stated *confidently* and *wrongly*, so they never appeared on
the gaps list. The gaps list only catches the unknowns you know about. It has been
rewritten to record only genuine, verified unknowns, and the two spec-internal
conflicts are now flagged at the head of the README rather than resolved silently.

**2026-07-30 — second-round corrections** (re-verified independently against rendered pages):

Two residual defects from the initial audit were fixed:

| # | Severity | Finding | File affected |
|---|---|---|---|
| A | **Severe** | `mabr-reporting.md` falsely claimed the per-event `object-delivery-status` (page 180) and the schema-level `MABR_ObjectDeliveryStatus` (page 183) were "the same enumeration". They are **not**: page 180 has 9 values incl. `cache-miss-timeshift`; page 183 has 10 values incl. `cache-miss-expired` (duplicated) but no `cache-miss-timeshift` and also no `cache-hit-mr`. TS 103 769 carries **three** mutually inconsistent versions of this enum. `MABR_ObjectDeliveryStatus` is an orphan type — never `$ref`'d. The `MABR_Report.required` quirk (`event` singular vs `events` plural) is also now documented. | `mabr-reporting.md` |
| B | **Severe** | Table A.0-1 v2 row under-listed the required namespaces. The real table requires **two**: `urn:dvb:metadata:Extensibility:2024` (Clause A.1) AND `urn:dvb:metadata:MulticastSessionConfiguration:2024` (Clause A.2). Fixed in both `mabr-signalling.md` §9 and the audit-history row above. | `mabr-signalling.md`, `README.md` |
| — | Policy | **Prose-vs-XSD flagging rule**: Where the prose table's Data type column disagrees with the XSD's `type="..."` value, the transcription follows the XSD (it is the normative wire-format authority) but **flags the divergence** so an implementer knows they may encounter tooling or documentation that expects the prose value. Finding 3 (`keyId`/`@keyUri`) and the updated finding 6 (`@protocolVersion`: `"String"` vs `xs:positiveInteger`) now both follow this rule uniformly. | — |

**What the audit confirmed correct** (independently verified against the PDF):
- The `dvb-flute` reuse claim is sound; Annex F's delta list is complete.
- V1.2.1 is the current edition; no later edition exists on the ETSI server.
- The core `MulticastTransportSession` element tree, rendezvous syntax,
  extensibility mechanism, FEC-scheme vocabulary, and Annex H deltas all check out.

Areas checked: every normative table/field in the three initial transcription files was
cross-read against a rendered page image or a table-aware `pdfplumber` extraction; the
entire Annex N YAML was extracted with `pdfplumber` (which preserves column order) and
verified field-by-field against clause 11 prose. The Annex A.0-1 table, Annex B.1
classification-scheme XML, Annex F.2.4.1 prose table, and Annex F.2.5.1 XSD were all
read directly and compared.

**Not yet independently verified (defer to a future audit):**
- The Annex A.2 baseline XSD beyond the types already spot-checked in the field tables.
- The Annex C.1 worked example XML (recommended as the primary round-trip fixture).
- The Annex I implementation guidance and Annex J NORM profile.
- The exact ROUTE packet-header bits (ATSC A/331 territory, blocked on issue #750).
