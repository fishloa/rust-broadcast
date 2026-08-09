# A/331 Annex A — ROUTE (Real-time Object delivery over Unidirectional Transport)

Source: ATSC A/331:2025-06, "Signaling, Delivery, Synchronization, and Error Protection", 18
June 2025, Annex A (pp. 144-185), plus the LCT/ALC field semantics ROUTE constrains from Annex
A §A.3.4/A.3.6 (which are themselves normatively RFC 5651/RFC 5775, not re-specified by A/331).
See [`README.md`](README.md) for provenance.

## 0. Reuse `dvb-flute` — do not re-implement LCT/ALC/FLUTE

**This workspace already has a crate for the layer ROUTE is built on.** `dvb-flute`
(`no_std` + `alloc`, depends only on `broadcast-common`) implements:

- `LctHeader` — the full RFC 5651 §5 LCT header (`V`/`C`/`PSI`/`S`/`O`/`H`/`A`/`B`, `HDR_LEN`,
  `CP`, plus the CCI/TSI/TOI variable-width fields) — see `dvb-flute/docs/lct.md`.
- `HeaderExtension` — the generic RFC 5651 §5.2 header-extension chain (variable-length HET
  0..=127 with HEL, fixed-length HET 128..=255 without) — see `dvb-flute/docs/lct.md`. This is
  already a generic `{ het: u8, content: &[u8] }` carrier, so **new HET values (see §2 below)
  need only a typed wrapper, not a new extension-chain parser.**
- `ExtTime` — RFC 5651 §5.2.2 EXT_TIME (HET=2), fully decoded (SCT-high/low, ERT, SLC) — see
  `dvb-flute/docs/lct.md`.
- `AlcPacket` + `EXT_FTI` (HET=64, RFC 5775) — see `dvb-flute/docs/alc.md`.
- `ExtFdt`/`ExtCenc` (HET=192/193, RFC 6726 FLUTE) — see `dvb-flute/docs/flute.md`.

**A ROUTE parser in the new `atsc3` crate should depend on `dvb-flute` for `LctHeader` and
`HeaderExtension`, and add only what ROUTE defines on top**: the ROUTE-specific field-value
constraints (§1 below), the two ROUTE-only extension headers (§2), the ROUTE FEC Payload ID
formats (§3), the Codepoint semantics (§4), the delivery-object model (§5), and the FEC
signalling/repair-flow XML model (§6-7) — all of which are genuinely new (not in `dvb-flute`
today, since that crate stops at generic ALC/FLUTE and does not know about ROUTE). This mirrors
how A/331 itself is written: Annex A is a **profile + delta** on RFC 5651/5775/6726, not a
ground-up restatement.

## 1. ROUTE's constraints on the LCT header (§A.3.4, §A.3.6)

ROUTE is "ALC (RFC 5775) with the following details" (§A.3.4) plus further LCT-field
constraints (§A.3.6, normative "shall"):

| LCT field | ROUTE-mandated value | Meaning |
|---|---|---|
| `V` (version, 4 bits) | `0001` | LCT version 1 (matches RFC 5651); "ROUTE version number" per A/331's own reading of this field. |
| `C` (2 bits) | `00` | CCI field is 32 bits; "may be set to 0" (CCI content itself unconstrained further). |
| `PSI` (2 bits) — **source packets** | `10` | ROUTE source protocol delivers only source packets, so PSI is fixed at `10`. |
| `PSI` (2 bits) — **repair packets** | first bit ("SPI", Source Packet Indicator) = `0` | §A.4.2.4: for repair packets, only the *first* bit of the 2-bit PSI field is given a ROUTE-defined meaning (0 = repair); the spec does not further constrain the second PSI bit for repair packets. |
| `S` (TSI flag, 1 bit) | `1` | TSI is a 32-bit word (`32*S + 16*H` with `H=0` => 32 bits). |
| `O` (TOI flag, 2 bits) | `01` | TOI is a 32-bit word (`32*O + 16*H` with `H=0` => 32 bits). |
| `H` (half-word flag, 1 bit) | `0` | No half-word field sizes. |
| `TSI` (32 bits) | — | Identifies the Transport Session (= LCT channel); scoped by sender IP; equals `S-TSID.RS.LS@tsi`. For repair flows, TSI identifies the *repair* flow (matched against the repair-flow's `@tsi` in the S-TSID). |
| `TOI` (32 bits) | — | Identifies the object within the session; mapping to the object is via the Extended FDT (source flows) or via the FEC super-object TOI (repair flows, unique per TSI). |
| `CP` (Codepoint, 8 bits) | see §4 below | ROUTE overloads CP to indicate delivery-object-format + fragmentation + ordering, either directly (CP 0-9) or by indirection through `SrcFlow.Payload@codePoint` (CP 128-255). |

Note (informative): §A.3.6 states ROUTE's one structural simplification versus generic
ALC/LCT — "ROUTE limits the usage of the LCT building block to a single channel per session"
— congestion control is therefore sender-driven; any receiver-driven layered-multicast
behavior is left to the application layer, not LCT itself.

## 2. ROUTE-specific LCT header extensions (§A.3.7, §A.3.8) — not in `dvb-flute`

Two new fixed/variable extensions are defined by A/331 itself (registered with IANA per the
spec text), neither of which exists in `dvb-flute` today:

### 2.1 EXT_ROUTE_PRESENTATION_TIME (HET = 66)
_§A.3.7.1, A/331:2025-06 p.166_

Variable-length extension (HET < 128, carries HEL). Present **only** in the first LCT packet of
an MDE (Media Delivery Event) data block containing a Random Access Point (RAP); "no other types
of LCT packets shall include" it. Presence of this header is itself the indicator that MDE mode
is in use for the stream.

| Syntax | No. of Bits | Format |
|---|---|---|
| `HET` | 8 | = 66 |
| `HEL` | 8 | uimsbf |
| `reserved` | 16 | — |
| `NTP timestamp, most significant word` | 32 | uimsbf |
| `NTP timestamp, least significant word` | 32 | uimsbf |

12 bytes total (Figure A.3.5). Carries the **full 64-bit NTP timestamp** of the presentation
time; value must always be greater than the accompanying EXT_TIME's SCT.

- **Companion requirement**: any LCT packet carrying EXT_ROUTE_PRESENTATION_TIME **shall** also
  carry RFC 5651's EXT_TIME (HET=2, already in `dvb-flute`) with both the SCT-High and SCT-Low
  flags set; the SCT value conveyed there denotes the "wait time" value
  `[EXT_ROUTE_PRESENTATION_TIME - SCT]` (§A.3.7.2, tied to §8.1.1.3's buffer model — not
  transcribed in this pass; see "could not establish").

### 2.2 EXT_TOL — Transport Object Length (§A.3.8.1)

Used by a ROUTE receiver (alongside, or instead of, RFC 5775's EXT_FTI, HET=64, already in
`dvb-flute`) to learn the delivery object's transfer length "after any content encoding (e.g.
gzip)". Two width variants, distinguished by HET:

#### 24-bit form — HET = 194 (fixed-length extension)
_Figure A.3.6, A/331:2025-06 p.167_

| Syntax | No. of Bits | Format |
|---|---|---|
| `HET` | 8 | = 194 |
| `Transfer Length` | 24 | uimsbf |

One 32-bit word total (matches the generic fixed-length HET>=128 shape already handled by
`dvb-flute`'s `HeaderExtension`: HET byte + 3 content bytes, no HEL).

#### 48-bit form — HET = 67 (variable-length extension)
_Figure A.3.7, A/331:2025-06 p.167_

| Syntax | No. of Bits | Format |
|---|---|---|
| `HET` | 8 | = 67 |
| `HEL` | 8 | uimsbf |
| `Transfer Length` | 48 | uimsbf, split across 2 32-bit words (16 bits in word 1, 32 bits in word 2) |

Two 32-bit words total (HEL = 2).

- **Selection rule**: "when EXT_FTI is not present, then either the 24-bit or 48-bit version of
  EXT_TOL should be present" (§A.3.8.1) — i.e. exactly one length-signalling mechanism
  (EXT_FTI *or* EXT_TOL) is expected per delivery object, not both, though the spec's language
  here is "should" (recommendation), not "shall".

## 3. FEC Payload ID formats (§A.3.5.1, §A.3.5.2)

The 32-bit "FEC Payload ID" word that follows the LCT header in every ALC/ROUTE packet
(`dvb-flute`'s `AlcPacket` already carries this as an opaque slice — RFC 5775 explicitly leaves
its format FEC-scheme-dependent) has **two concrete ROUTE-defined layouts**:

### 3.1 Source flows — Compact No-Code FEC scheme
_Figure A.3.3, A/331:2025-06 p.163_

| Syntax | No. of Bits | Format |
|---|---|---|
| `start_offset` | 32 | uimsbf |

A single 32-bit unsigned integer: the octet offset, from the first octet of the delivery
object, of the first octet of the fragment carried in *this* packet's payload. (§A.3.4: value
`0` when the packet carries the entire object.) This directly matches `SrcFlow.Payload
@srcFecPayloadId = 0` semantics (see §5 below).

### 3.2 Repair flows — RaptorQ FEC scheme (RFC 6330)
_Figure A.3.4, A/331:2025-06 p.164_

| Syntax | No. of Bits | Format |
|---|---|---|
| `SBN` (Source Block Number) | 8 | uimsbf |
| `Encoding Symbol ID` | 24 | uimsbf |

**Corrected 2026-08-09**: this table previously read SBN 16 / ESI 16, which is wrong and
contradicted its own citation. A/331:2026-04 §A.3.5.2 states the layout is "In accordance with
RFC 6330 [28] Section 3.2", and Figure A.3.4's bit ruler gives SBN 8 bits, ESI 24 bits —
matching RFC 6330 exactly. Verified by counting the figure's own bit-diagram columns in the
vendored PDF.

Per RFC 6330 §3.2. **RFC 6330 is not vendored in this repository** (neither `specs/` nor
`private/specs/`); the SBN/ESI field widths above come from A/331's Figure A.3.4, but
the RaptorQ *encoding/decoding* procedure itself (how source/repair symbols are generated from
these identifiers) is out of scope of A/331 and not established here — see `README.md`.

## 4. Codepoint (CP) semantics (§A.3.6, Table A.3.6)

The CP field indicates the type of delivery object carried, and for `CP >= 128` indirects
through the `SrcFlow.Payload@codePoint`-matched element (§7.1's Table A.3.1 `Payload` element,
see §5 below) to learn `@formatId`/`@frag`/`@order`.

### Table A.3.6 — Defined Values of Codepoint Field of LCT Header
_§A.3.6, A/331:2025-06 p.164-165_

| CP value | Semantics | `@formatId` | `@frag` | `@order` |
|---|---|---|---|---|
| 0 | ATSC Reserved (not used) | — | — | — |
| 1 | NRT — File Mode | 1 (File Mode) | 0 (arbitrary) | true |
| 2 | NRT — Entity Mode | 2 (Entity Mode) | 0 | true |
| 3 | NRT — Unsigned Package Mode | 3 (Unsigned Package Mode) | 0 | true |
| 4 | NRT — Signed Package Mode | 4 (Signed Package Mode) | 0 | true |
| 5 | New Initialization Segment (IS), timeline changed | 1 (File Mode) | 0 | true |
| 6 | New IS, timeline continued | 1 | 0 | true |
| 7 | Redundant IS | 1 | 0 | true |
| 8 | Media Segment, File Mode | 1 | 1 (sample-based) | true |
| 9 | Media Segment, Entity Mode | 2 (Entity Mode) | 1 | true |
| 10-127 | ATSC Reserved | — | — | — |
| 128-255 | Attributes given by the `SrcFlow.Payload` element whose `@codePoint` matches this CP value | per-element | per-element | per-element |

`@frag` values (§A.3.6 detailed text, referenced from the `SrcFlow.Payload` table in §5):
`0` = arbitrary byte-boundary fragmentation; `1` = application-specific, sample-based (one or
more complete ISOBMFF samples per ISO/IEC 14496-12, used for MDE mode carrying an `mdat` box);
`2` = application-specific, box-collection (one or more complete ISOBMFF boxes starting with a
RAP, e.g. `styp`/`sidx`/`moof`, used for MDE mode); `3-255` = ATSC Reserved.

Note (informative): CP values 5-9 all pertain to delivering a sequence of related objects for a
DASH stream and exist specifically to support **MPD-less start-up** — a receiver can begin
playback from these CP-tagged objects before it has acquired a complete MPD.

## 5. Delivery object model (§A.3.3)

A/331 defines four delivery-object encodings, selected by `SrcFlow.Payload@formatId`:

### Table A.3.2 — Meaning of the Delivery Object Format ID Values
_§A.3.3.1, A/331:2025-06 p.153_

| Format ID | Meaning |
|---|---|
| 0 | ATSC Reserved |
| 1 | File Mode (§A.3.3.2) |
| 2 | Entity Mode (§A.3.3.3) |
| 3 | Unsigned Package Mode (§A.3.3.4) |
| 4 | Signed Package Mode (§A.3.3.5) |
| >=5 | ATSC Reserved |

- **File Mode (1)** — a complete file or byte-range, described by an **Extended FDT** (ATSC's
  extension of the RFC 6726 FLUTE FDT — base FDT attributes `Content-Location`,
  `Transfer-Length`, `FEC-OTI-*`, `Expires` come from RFC 6726 §3.4.2, already vendored at
  `specs/rfc6726_flute.txt`, not re-transcribed here). The Extended FDT is delivered either (a)
  embedded as the `SrcFlow.EFDT` element in the S-TSID, or (b) as a separate delivery object
  with **TOI=0** in the same LCT channel — this is FLUTE-compatible, matching `dvb-flute`'s
  documented "TOI=0 is reserved for FDT Instances" FLUTE convention. ATSC adds its own FDT
  extensions (`@efdtVersion`, `@maxExpiresDelta`, `@maxTransportSize`, `@appContextIdList`,
  `@fileTemplate`, `@filterCodes`, `@maxCacheMemory`, `@order` on FDT-Instance; `@appContextIdList`/
  `@filterCodes` on FDT-Instance.File — Tables A.3.3/A.3.4, §A.3.3.2.3) plus reuses 3GPP MBMS
  FDT extensions (`Base-URL-1/2`, `Cache-Control`, `Alternate-Content-Location-1/2` —
  §A.3.3.2.4/.5; **3GPP TS 26.346 is not vendored in this repo**, so those elements' full
  syntax/semantics are only as summarized by A/331 itself, not independently verified).
  - `@fileTemplate` substitution: the string must contain the literal `$TOI$` token (optionally
    suffixed `%0[width]d` for zero-padding), substituted with the packet's TOI value to derive
    `Content-Location` on the fly — avoids continuously re-sending the FDT for real-time
    objects. Table A.3.5 defines the substitution grammar (`$$` -> literal `$`, `$TOI$` ->
    TOI value, default width 1).
  - **ROUTE-vs-FLUTE delta (§A.3.3.2.7)**: in File Template mode, EXT_TOL/EXT_FTI presence is
    upgraded from the general "should" (§A.3.8.1) to "shall": if `File@Transfer-Length` in the
    Extended FDT Instance is not present, then EXT_TOL or EXT_FTI *shall* be present. If the
    broadcaster does not know the length at the start of the transfer, EXT_TOL or EXT_FTI shall
    be included in at least the last packet (and *should* be in the last few packets). This is
    stricter than the base FLUTE/ALC rule where EXT_FTI is optional.
- **Entity Mode (2)** — delivery object modeled as an HTTP/1.1 representation: entity/payload/
  response headers (RFC 7231 §§3.1, 3.3, 7) accompany the object instead of an FDT. A
  `Content-Range` header indicates a byte-range portion of a larger target file.
- **Unsigned Package Mode (3)** — a group of files packaged as `multipart/related` (RFC 2387).
  `SrcFlow.Payload@formatId = "3"`; binary files should use `Content-Transfer-Encoding: binary`.
- **Signed Package Mode (4)** — as above but `multipart/signed` (S/MIME, RFC 5751), for
  packages needing a validation signature (e.g. broadcaster application code, ATSC A/360's
  "Application Code Signing" / "Signatures for Service Layer Signaling").

### `SrcFlow` element (declares a source flow within an S-TSID LCT channel)

`SrcFlow` is a child of `S-TSID.RS.LS` (see [`a331-signalling.md`](a331-signalling.md) for the
enclosing S-TSID structure).

#### Table A.3.1 — XML Format of SrcFlow Element
_§A.3.2.1, A/331:2025-06 p.148-149_

| Element / Attribute | Use | Data Type | Description |
|---|---|---|---|
| `SrcFlow` | | `srcFlowType` | Source flow carried in the LCT channel. |
| `@rt` | 0..1 | boolean | Real-time content indication. Default `"false"`. |
| `@minBufferSize` | 0..1 | unsignedInt (32-bit) | Minimum receiver transport buffer size, in kilobytes. Applicable only when `@rt="true"`. |
| `EFDT` | 0..1 | — | Embedded Extended FDT Instance (mutually exclusive with a TOI=0 FDT-Instance object in the same channel). |
| `ContentInfo` | 0..1 | — | Additional content metadata; a channel should carry only one content type. |
| `ContentInfo.MediaInfo` | 0..1 | — | DASH Representation info. |
| `MediaInfo@startup` | 0..1 | boolean | MPD-less start-up eligibility. Default `"false"`. |
| `MediaInfo@lang` | 0..1 | lang | Deprecated — backwards-compat only; decoders should ignore. |
| `MediaInfo@contentType` | 0..1 | contentType | One of `audio`/`video`/`subtitles`. |
| `MediaInfo@repId` | 1 | StringNoWhitespace | DASH Representation ID. |
| `ContentRating` | 0..N | — | Deprecated — use `MPD.Period.AdaptationSet.Rating`. |
| `ContentInfo.AEAMedia` | 0..1 | — | Container of AEA (emergency alert) message identifiers. |
| `AEAMedia.AEAId` | 1..N | string | Identifier of an associated AEA message. |
| `ContentInfo.Payload` | 0..N | — | Payload attributes for a given Codepoint (CP) value; required for CP in 128-255, implicit (Table A.3.6) for CP < 128. |
| `Payload@codePoint` | 1 | unsignedByte (128-255) | The CP value this Payload element describes. |
| `Payload@formatId` | 1 | unsignedByte | Delivery object format (Table A.3.2). |
| `Payload@frag` | 0..1 | unsignedByte | Fragmentation mode (0/1/2 — see §4 above). Default `0`. |
| `Payload@order` | 0..1 | boolean | In-generation-order delivery indication. Default `true`. |
| `Payload@srcFecPayloadId` | 0..1 | unsignedByte | FEC Payload ID interpretation: `0` = 32-bit start_offset (§3.1 above); `1-255` = ATSC Reserved. Default `0`. |

## 6. FEC framework overview (§A.4.1-A.4.2) — the repair protocol

A/331's FEC framework layers on RFC 6363 (FECFRAME) concepts and RFC 5052 (the FEC building
block already underlying FLUTE/ALC), but protects **delivery objects**, not raw packets, and
supports bundling multiple objects into one FEC-protected unit.

Key constructions (all normative, §A.4.2):

1. **FEC transport object** — one delivery object's associated FEC-protectable unit: the
   concatenation of the delivery object's `F` octets, `P` padding octets, and a trailing 4-octet
   big-endian field `f` carrying `F` itself. Total size `S = ceil((F+4)/Y)` symbols (`Y` = FEC
   symbol size), so `P = S*Y - 4 - F`. Padding and the trailing size field are **never sent** in
   source packets — they exist only inside the FEC domain, recovered by FEC decoding.
2. **FEC super-object** — the concatenation of `N` FEC transport objects (`N >= 1`), in
   numerical order, total size `K = sum(S[i])` symbols. Metadata needed per transport object
   inside a super-object: source TSI, source TOI, start octet, and size in symbols.
3. **Repair packet structure** (§A.4.2.4) — an ALC/LCT packet where: TSI identifies the repair
   flow (matched against the repair flow's `@tsi`, see §7 below); the PSI "SPI" bit = `0` (see
   §1 table above); the repair object/super-object TOI (unique per repair-flow TSI) is carried
   per RFC 6330's FEC building block, but **only repair packets are ever sent** (no systematic
   source symbols are retransmitted through the repair flow — those come from the source flow
   itself).
4. **Summary FEC information needed per super-object** (§A.4.2.5): FEC OTI (RFC 5052) + Table
   A.4.1's additional fields (below); the object count `N`; and per transport object, its
   source TSI/TOI, start octet, and symbol-size `S`. Delivered statically in the `RepairFlow`
   XML declaration (§7) and/or dynamically via an LCT extension header (mechanism/HET not
   further specified by A/331 in the text transcribed for this pass — see "could not
   establish").

## 7. Repair flow declaration — the `RepairFlow` element

`RepairFlow` is a child of `S-TSID.RS.LS`, a sibling of `SrcFlow` (§6.1.4 in
[`a331-signalling.md`](a331-signalling.md)).

### Table A.4.1 — Semantics of RepairFlow Element
_§A.4.3.2, A/331:2025-06 p.175-177_

| Element / Attribute | Use | Data Type | Description |
|---|---|---|---|
| `RepairFlow` | | `stsid:rprFlowType` | Repair flow carried in the LCT channel. |
| `FECParameters` | 0..1 | — | FEC parameters for this repair flow. |
| `FECParameters@maximumDelay` | 0..1 | unsignedInt (32-bit) | Max delivery delay (ms) between a source packet and its corresponding repair packet. No default when absent. |
| `FECParameters@overhead` | 0..1 | unsignedShort (16-bit, 0-1000%) | AL-FEC-related-field overhead as a percentage of repair-packet size (sum of `@maximumDelay`, `@overhead`, `@minBuffSize`, `@fecOTI`). |
| `FECParameters@minBuffSize` | 0..1 | unsignedInt (32-bit) | Required receiver AL-FEC-decode buffer size (bytes) to handle all objects assigned to one super-object. |
| `FECParameters@fecOTI` | 1 | hexBinary (12 octets) | Concatenated Common + Scheme-Specific FEC OTI per RFC 6330 §§3.3.2-3.3.3. The 40-bit Transfer Length subfield may be all-zero to mean "unknown/streaming" (live or size-variable pre-recorded content). |
| `FECParameters@percentRepair` | 0..1 | unsignedShort (0-200) | Max ratio of repair symbols to source symbols, as a percentage. Absence = not provided. |
| `FECParameters@checksumList` | 0..1 | space-separated hex list | Per-source-block CRC32 checksums, in source-block order. |
| `ProtectedObject` | 0..N | — | One source flow protected by this repair flow, and how. |
| `ProtectedObject@sessionDescription` | 0..1 | string (comma-separated `key=value` list) | Session description of the protected source flow, when it is *not* carried in the same ROUTE session/LCT channel as the repair flow (e.g. `S-TSID.RS@sIpAddr`, `@dIpAddr`, `RS.LS@dPort`, `@tsi`, `@bw`, `@startTime`, `@endTime` — list not exhaustive). Absence implies same channel. |
| `ProtectedObject@tsi` | 1 | unsignedInt (32-bit) | TSI of the protected source flow. |
| `ProtectedObject.SourceTOI` | 1..N | — | TOI value(s) of the protected source object(s), or the `X`/`Y` mapping constants. |
| `SourceTOI@x` | 0..1 | unsignedShort | Constant `X` in `sourceTOI = X*TOI + Y`. Default `1`. |
| `SourceTOI@y` | 0..1 | unsignedShort | Constant `Y` in the same equation. Default `0`. |

**TOI mapping (§A.4.3.3, normative)**: given a repair packet's TOI, the corresponding protected
delivery object's TOI is `sourceTOI = X*TOI + Y` (a C-language-format equation with at most one
variable, `TOI`, no trailing `;`). When no `SourceTOI` element is present at all, the default
equation `sourceTOI = TOI` applies (i.e. `X=1, Y=0`, a 1:1 mapping between the repair
(super-)object's TOI and the protected delivery object's TOI).

Note: the LCT channel's own TSI (carrying the repair flow) is **not** an attribute of
`RepairFlow` — it is already given by `S-TSID.RS.LS@tsi` for the LCT channel the `RepairFlow`
element is nested under (see [`a331-signalling.md`](a331-signalling.md) §6.1.4).

## 8. Sender/receiver operational procedures — deferred detail

§A.3.9 (Basic ROUTE Sender Operation), §A.3.10 (Basic ROUTE Receiver Operation), §A.4.4 (Repair
Receiver Operation) and §A.4.5 (worked repair example) describe operational algorithms/state
machines for packetizing and reassembling delivery objects, and §A.4.3.4 walks five worked
`RepairFlow` XML examples. These are implementation guidance built entirely on the wire
structures already fully captured above (LCT/FEC fields, `SrcFlow`/`RepairFlow` XML, FEC
transport/super-object construction) — this pass did not additionally transcribe their
narrative/example text verbatim, since no new wire-format fields appear there. An implementer
should read those clauses directly in A/331 before writing the reassembly/sender state machine;
see `README.md`'s "could not establish" list.

## Overlap with other workspace crates/issues

- **`dvb-flute`** (this workspace) already implements the RFC 5651/5775/6726 layer ROUTE sits
  on — see §0 above. **Do not re-implement LCT/ALC/FLUTE parsing in the new `atsc3` crate.**
- **Issue #755 (DVB-MABR)** covers ETSI TS 103 769's multicast ABR stack, which is *also* built
  on FLUTE/ALC/LCT (and, per `dvb-flute`'s own README, is one of the reasons that crate exists:
  "the building blocks beneath DVB-IPTV / DVB-MABR file delivery"). ROUTE and DVB-MABR are
  sibling profiles of the same RFC 5651/5775/6726 base, so both should depend on `dvb-flute`
  rather than each growing their own LCT/FEC-header code — whichever of #750/#755 lands first
  should make sure any genuinely-shared-but-currently-missing piece (e.g. a generic FEC
  transport-object/super-object helper, if DVB-MABR turns out to need the same construction)
  goes into `dvb-flute` or `broadcast-common`, not duplicated.
- **`transmux`** already handles DASH/HLS/CMAF packaging and CENC. ROUTE's payload, once
  reassembled from LCT/FEC framing, is typically DASH-formatted CMAF Segments/Initialization
  Segments (see CP values 5-9 in §4) — the *reassembly* (ROUTE -> delivery object bytes) belongs
  in the new `atsc3` crate, but once a full ISOBMFF/CMAF object is recovered, its *container*
  parsing is `transmux`'s job, not a reason to duplicate ISOBMFF parsing inside `atsc3`.
