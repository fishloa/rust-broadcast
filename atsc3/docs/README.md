# atsc3 — spec table reference (prep for issue #750)

This is **prep work only** — there is no `atsc3` crate yet (no `Cargo.toml`, no `src/`). This
`docs/` directory transcribes the ATSC 3.0 spec clauses a future `atsc3` crate implementation
would need, per this repo's spec-grounding discipline (every wire layout must cite a spec
section; syntax tables live as reviewable markdown before any code is written).

## Sources and provenance

| Document | Revision | Obtained from | Date fetched |
|---|---|---|---|
| ATSC A/331, "Signaling, Delivery, Synchronization, and Error Protection" | **A/331:2025-06** (18 June 2025; a rollup of A/331:2025-02 Amendment No. 1). Also verified against **A/331:2026-04** (14 April 2026) — no differences in transcribed clauses. | `https://www.atsc.org/wp-content/uploads/2025/06/A331-2025-06-Signaling-Delivery-Sync-FEC.pdf` | 2026-07-29 (this pass); 2026-07-30 (2026-04 verification) |
| ATSC A/321, "System Discovery and Signaling" | **A/321:2026-06** (11 June 2026) | `https://www.atsc.org/wp-content/uploads/2026/06/A321-2026-06-System-Discovery-and-Signaling.pdf` | 2026-07-29 (this pass) |

Both documents were checked for a private-submodule copy first
(`private/specs/*.pdf` after `git submodule update --init private`) — **neither is vendored
there**; the submodule has `atsc_a342-3_2025_mpegh_system.pdf` and
`atsc_a53_part4_2009_mpeg2_video.pdf` but no A/331 or A/321. Both are freely downloadable from
`atsc.org` (confirmed via `curl` — no paywall, no login), so they were fetched directly from
the URLs above rather than deferred. Neither PDF has been added to this repo (public or
private) — only their extracted markdown lives under a scratch directory used to write these
docs; re-running the same pipeline against either URL will reproduce it. The 2026-04 edition of
A/331 was separately fetched and verified (see "A/331 revision note" below).

**A/331 revision note**: A/331:2025-06 was the version transcribed. A/331:2026-04 (14 April
2026, "references to ATSC documents updated") was subsequently fetched and diffed against
2025-06 across every clause transcribed here (Annex A ROUTE tables, §6 LLS/SLT tables, §7.1
ROUTE/DASH SLS tables, §7.2 MMT envelope + USBD tables, Annex H media-type registry, Annex G.2
Wake-up Field table). **No value, bit-width, or semantic differs in any transcribed clause.**
Both revisions carry the same wire format. The transcription remains current.

**A/321 revision note**: **A/321:2026-06** was the latest found (search results also mentioned
a "2025-07" filename that 404s — likely a search-index artifact, not a real prior revision at
that URL; 2026-06 is confirmed live and is what was transcribed).

Both PDFs were converted to Markdown locally with the `pdf2md` skill/tool
(`~/Projects/pdf2md`, `--engine textlayer`, both are native-text PDFs, not scanned), which
diffs extracted numeric/hex tokens against the PDF's own text layer as a corruption check.
Conversion exit codes: A/321 full-document conversion = **0** (clean, no flagged tokens); A/331
Annex A (pp. 158-199), §6 (pp. 39-66), §7.2 (pp. 92-135), Annex H (pp. 246-252) = **0** each; A/331
§7.1 (pp. 67-92) = **non-zero**, flagging two token-count mismatches (`0x1002` and `0x01`, each
appearing a different number of times between engine output and text layer). Both were manually
checked against the converted Markdown and against the source page images' rendering pattern:
in both cases the *value* is correct and present in the output, and the mismatch is an
occurrence-count artifact from a table row that visually repeats the same value across a
page-break re-flow, not a dropped or corrupted digit. No hex/decimal value in the tables below
was taken from a location the verifier flagged as actually altered.

**RFC cross-references used without re-transcription**: ROUTE (A/331 Annex A) explicitly builds
on IETF RFC 5651 (LCT), RFC 5775 (ALC), and RFC 6726 (FLUTE). All three are already vendored,
freely-redistributable text in this repo's public `specs/` directory
(`rfc5651_lct.txt`, `rfc5775_alc.txt`, `rfc6726_flute.txt`) and are **already implemented** by
the existing `dvb-flute` crate (with its own `docs/{lct,alc,flute,norm}.md` transcriptions).
[`a331-route.md`](a331-route.md) reads those RFCs directly (not just A/331's summary of them)
for the base LCT header/extension format, and points to `dvb-flute` rather than re-transcribing
what that crate's docs already cover. RFC 6330 (RaptorQ, referenced by A/331 for repair-flow FEC
Payload IDs) and 3GPP TS 26.346 (MBMS, referenced by A/331 for FDT extensions) are **not**
vendored anywhere in this repo and were not independently fetched for this pass — see "could not
establish" below.

## Files in this directory

- [`a321-bootstrap.md`](a321-bootstrap.md) — A/321 system-discovery bootstrap: fixed
  physical-layer signal dimensions, major/minor versioning (ZC root + PN seed), per-symbol
  signaling-bit fields for major versions 0 and 1, multiplexing of signal types.
- [`a331-route.md`](a331-route.md) — **the load-bearing document.** A/331 Annex A's ROUTE
  profile of LCT/ALC/FLUTE: ROUTE's LCT-field constraints, the two ROUTE-only extension headers
  (EXT_ROUTE_PRESENTATION_TIME, EXT_TOL), the two ROUTE FEC Payload ID formats, the Codepoint
  semantics table, the delivery-object model (File/Entity/Unsigned-Package/Signed-Package
  modes) and the `SrcFlow` XML element, and the FEC repair framework (FEC transport
  object/super-object construction, the `RepairFlow` XML element, TOI mapping). Opens with an
  explicit pointer to reuse the existing `dvb-flute` crate for the underlying LCT/ALC/FLUTE
  layer rather than re-implementing it.
- [`a331-signalling.md`](a331-signalling.md) — LLS (transport, `LLS_table()` envelope, SLT) and
  ROUTE/DASH SLS (USBD, S-TSID, MPD pointer, APD, HELD, DWD, RSAT pointer), plus the Annex H
  media-type registry.
- [`a331-mmt.md`](a331-mmt.md) — the MMT signalling path, **partially transcribed with an
  explicit scope statement**: the ATSC-original `mmt_atsc3_message()` envelope and the MMTP USBD
  fragment are fully transcribed; the further catalog of video/audio/caption/DRM descriptor
  bit-tables (A/331 §7.2.3.2-§7.2.4) is listed by clause number but not transcribed, and the
  base MMT container format (MP table, MPU, MMTP packetization) is explicitly deferred because
  it belongs to **ISO/IEC 23008-1**, a paywalled ISO standard not present in this repository.

## Wire structures documented (count)

Counting each distinct bit-syntax table or XML element/attribute table transcribed with a
clause citation:

- A/321: 3 per-symbol signaling-field tables (bootstrap symbols 1/2/3) + 2 PN-seed lookup tables
  (major versions 0 and 1) + 1 minimum-time-interval lookup table = **6**.
- A/331 Annex A (ROUTE): LCT field-constraint table, 2 new extension-header layouts
  (EXT_ROUTE_PRESENTATION_TIME, EXT_TOL x2 widths), 2 FEC Payload ID layouts, 1 Codepoint
  semantics table, 1 delivery-object-format-ID table, 1 `SrcFlow` XML table, 1 `RepairFlow` XML
  table = **9**.
- A/331 §6 (LLS/SLT): `LLS_table()` envelope, SLT XML table + 4 code-value tables
  (`urlType`, `serviceCategory`, `slsProtocol`, `OtherBsid@type`) = **6**.
- A/331 §7.1 (ROUTE/DASH SLS): USBD, S-TSID, APD, HELD, DWD XML tables = **5**.
- A/331 §7.2 (MMT, partial): `mmt_atsc3_message()` bit-syntax + 2 code-value tables + MMTP USBD
  XML table = **4** (fully transcribed); ~15 further descriptor tables cataloged by clause
  number but **not** transcribed.
- Annex H media-type registry: 1 summary table (6 fragment media types) = **1**.

**Total: 31 wire structures/tables fully transcribed with clause citations**, plus one
explicitly-scoped catalog (MMT's remaining descriptors) left for future work.

## Overlaps flagged for the implementer

- **`dvb-flute`** (this workspace) already implements RFC 5651 LCT, RFC 5775 ALC (incl. EXT_FTI),
  and RFC 6726 FLUTE (incl. EXT_FDT/EXT_CENC, the TOI=0 convention). The `atsc3` crate's ROUTE
  layer should **depend on `dvb-flute`**, not duplicate its LCT/header-extension parsing. See
  [`a331-route.md`](a331-route.md) §0.
- **Issue #755 (DVB-MABR)** — ETSI TS 103 769 is a sibling multicast/ROUTE-like stack over the
  same FLUTE/ALC/LCT base (this is in fact part of why `dvb-flute` exists as a shared crate per
  its own README). Whichever of #750/#755 lands a genuinely-shared helper (e.g. FEC
  transport-object/super-object construction, if DVB-MABR needs the same shape) should land it
  in `dvb-flute`/`broadcast-common`, not duplicate it.
- **`transmux`** already parses DASH MPD and handles CENC (ISO/IEC 23001-7) for ISOBMFF/CMAF.
  A/331's MPD fragment is DASH-IF's MPD verbatim, and A/331's MMT DRM signalling maps onto the
  same CENC model `transmux` already has. The new `atsc3` crate's job is recognizing/routing
  these payloads, not re-implementing MPD or CENC parsing.

## Could not establish (explicit, not guessed)

- **A/321**: the concrete bootstrap-version-to-signal-type allocation
  registry (beyond A/321's own two illustrative example tables, 7.1/7.2) was not located —
  presumably the ATSC Code Point Registry, an external, evolving document.
- **A/331 Annex A (ROUTE)**: RFC 6330 (RaptorQ) is not vendored in this repo, so the repair
  flow's FEC Payload ID (SBN + Encoding Symbol ID) field widths are transcribed from A/331's own
  figure, but the RaptorQ encoding/decoding *procedure* itself is out of scope of A/331 and not
  established here. 3GPP TS 26.346 (MBMS) is likewise not vendored, so the 3GPP-defined FDT
  extensions (`Base-URL-1/2`, `Cache-Control`, `Alternate-Content-Location-1/2`) are only as
  summarized by A/331's own brief text, not independently verified against the base 3GPP
  document. The dynamic (LCT-extension-header-carried) form of FEC summary information
  mentioned in §A.4.2.5 is referenced but its concrete header layout is not spelled out in the
  clauses read for this pass.
- **A/331 §6 (LLS)**: §6.4 (System Time fragment), §6.5 (Advanced Emergency Alerting Table),
  §6.6 (OnscreenMessageNotification), §6.7 (SignedMultiTable), §6.8 (UserDefined), and Annex F
  (Rating Region Table) were not read/transcribed in this pass — flagged inline in
  [`a331-signalling.md`](a331-signalling.md) rather than guessed at. Annex C ("Filtering for
  Signaling Fragments", the TOI-encoding rule for SLS-fragment-type filtering) was likewise not
  transcribed.
- **A/331 §7.2 (MMT)**: the entire MP table / MPU / MMTP packet/session model is defined by
  ISO/IEC 23008-1, which is not vendored in this repo and was not independently obtained (it is
  a paywalled ISO standard, unlike the freely-published ATSC/IETF documents used elsewhere in
  this pass). Everything in [`a331-mmt.md`](a331-mmt.md) about MP-table/MPU/MMTP-packet
  structure is at most a paraphrase of A/331's own references to those ISO clauses, never an
  independent transcription of ISO/IEC 23008-1's normative syntax. Within A/331 itself, the
  video/audio/caption stream-properties descriptors, the Staggercast descriptor, and the
  CENC/DRM descriptors (§7.2.3.2-§7.2.4, roughly 1,500 lines of source PDF, Tables 7.12-7.39)
  were read enough to catalog by clause number (see [`a331-mmt.md`](a331-mmt.md) §4) but not
  transcribed field-by-field.
- **The `.xsd` XML schema files** A/331 cites throughout (e.g. `SLT-1.0-20211209.xsd`,
  `S-TSID-1.0-20230714.xsd`, `ROUTEUSD-1.0-20170920.xsd`, `MMTUSD-1.0-20210401.xsd`,
  `APD-1.0-20170209.xsd`, `HELD-1.0-20210312.xsd`, `DWD-1.0-20180830.xsd`,
  `ATSC-FDT-1.0-20230714.xsd`) are distributed by ATSC as a separate zip archive
  (`atsc.org/standards`), not embedded in the PDF text, and were not fetched in this pass. All
  element/attribute tables in this directory come from A/331's own *informative* prose tables
  plus the accompanying normative "shall" text — described by A/331 as informative
  restatements of the (unfetched) normative schemas, so treat the schemas as the final
  authority if a discrepancy is ever found.

## Adversarial fidelity audit (2026-07-30)

Every numeric/bit-syntax table was independently verified against its source PDF prior to commit,
and a second adversarial audit was done against the same source with the following scope:

**Tables verified** (no fabrication found in any):
- A/321 Tables 6.1 (PN seed per minor version), 6.2-6.5 (bootstrap symbol 1/2/3 signaling
  fields + symbol 2), 6.6 (major version 1 PN seeds)
- A/331 Annex A Tables A.3.1 (SrcFlow XML), A.3.2 (delivery-object format IDs), A.3.3/A.3.4
  (EFDT extensions), A.3.5 (file-template identifiers), A.3.6 (Codepoint semantics), A.4.1
  (RepairFlow XML)
- A/331 Tables 6.1 (LLS_table() bit-syntax), 6.2 (SLT XML), 6.3-6.5 (code-value tables), 6.6
  (OtherBsid@type), 7.9 (mmt_atsc3_message()), 7.10-7.11
- Annex H media-type registry

**Defects found and corrected:**
1. **Reference [2] misreported as unvendored** — A/321's ref [2] is A/331:2026-04 (named
   explicitly in A/321's bibliography §2.1). The `ea_wake_up_1`/`ea_wake_up_2` semantics were
   always present in the A/331 PDF already transcribed — the gap was a research error, not a
   sourcing one. Fixed: ref [2] identified, A/331 Annex G.2's Wake-up Field table + semantics
   transcribed into [`a321-bootstrap.md`](a321-bootstrap.md) §7, and the README gaps list
   updated.
2. **A/331 revision currency** — A/321:2026-06 cites A/331:2026-04; this PR transcribed
   2025-06. A/331:2026-04 was fetched and diffed against 2025-06 across every transcribed
   clause. **No value, bit-width, or semantic differs in any transcribed clause.** The
   transcription remains current. The README source table now names both revisions.
3. **Wrong cross-reference** — EXT_TOL "should be present" selection rule cited "§A.3.5.1 area"
   in [`a331-route.md`](a331-route.md); it lives in §A.3.8/A.3.8.1. Fixed.
4. **Missing ROUTE-vs-FLUTE delta** — §A.3.3.2.7 (File Template mode) upgrades
   EXT_TOL/EXT_FTI presence from the general "should" of §A.3.8.1 to "shall", with a
   last-packet timing requirement. This was omitted from the delta list. Added.
