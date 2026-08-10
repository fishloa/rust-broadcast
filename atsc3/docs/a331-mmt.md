# A/331 §7.2 — MMTP-Specific Service Layer Signaling

Source: ATSC A/331:2025-06 §7.2 (pp. 78-121). See [`README.md`](README.md) for provenance.

## Scope decision: partial transcription, explicit deferral for the rest

This document is **deliberately not at the same level of completeness as
[`a331-route.md`](a331-route.md) / [`a331-signalling.md`](a331-signalling.md)**, for two
concrete reasons, not laziness:

1. **The MMT container format itself is not A/331's to specify, and is not in this repo.**
   A/331's MMTP-specific signaling is a thin ATSC layer on top of **ISO/IEC 23008-1 (MPEG Media
   Transport, "MMT")** — the MP (MMT Package) table, MPU structure, MMTP packet/session model,
   and the base signaling-message framing all live in ISO/IEC 23008-1, a paywalled ISO standard.
   Unlike ROUTE (which builds on freely-available IETF RFCs 5651/5775/6726, already vendored in
   this repo and already implemented by `rmt-flute`), **a parser cannot be spec-complete for
   MMT without ISO/IEC 23008-1**, which this repository does not have and this pass could not
   obtain (it is not freely published the way the ATSC standards or IETF RFCs are). Anything
   this document says about MP-table/MPU/MMTP-packet structure is at most a paraphrase of what
   A/331 says *about* those ISO structures, not a transcription of their normative syntax.
2. **Within what A/331 *does* fully specify itself** (the ATSC-original
   `mmt_atsc3_message()` envelope and its content-type registry, §7.2.3.1, plus the MMTP USBD
   fragment, §7.2.1) — this document transcribes those completely (§1-§3 below). What is
   explicitly **not** transcribed is the further catalog of ATSC-defined *descriptor* payloads
   carried inside that envelope (video/audio/caption stream-properties descriptors, the
   Staggercast descriptor, and the CENC/DRM descriptors, §7.2.3.2-§7.2.4) — roughly 1,500 lines
   of dense bit-tables (A/331's own Tables 7.12-7.39) covering mostly codec-parameter and
   DRM-key-signalling detail. §4 below lists exactly what exists there, with clause numbers, so
   a future pass can transcribe them without re-discovering the table of contents; they were
   deprioritized against the ROUTE path (issue #750's own framing: "A/331 ROUTE + MMT
   signalling" — ROUTE-first) since ROUTE/DASH is the dominant ATSC 3.0 deployment path and MMT
   is comparatively rare in the field.

## 1. MMTP session content — what rides an MMTP session (§7.2, overview, informative + normative mix)

Per the SLT (`BroadcastSvcSignaling@slsProtocol=2`), an MMTP-delivered Service's SLS comprises:
the **USBD** fragment (§3 below), the **MP (MMT Package) table** (external — ISO/IEC 23008-1
§10.3.4, referenced not reproduced), the **DWD** fragment (identical to the ROUTE one, see
[`a331-signalling.md`](a331-signalling.md) §4.6), the **HELD** fragment (identical to the
ROUTE one, see [`a331-signalling.md`](a331-signalling.md) §4.5), and the **RSAT** fragment
(external, A/200). For hybrid delivery, an **MPD** fragment (DASH-IF, external) may also be
present for broadband-delivered Components.

Messages required on the MMTP session signaled in the SLT (§7.2.3, normative "shall"):

- **MPT (MMT Package Table) message** — carries the MP table (ISO/IEC 23008-1 §10.3.4): the
  list of Assets (Service Components) and their location info.
- **MA3 (`mmt_atsc3_message()`) message** — carries ATSC-specific system metadata; see §2 above.

Required-if-needed:

- **MPI (Media Presentation Information) message** — carries an MPI table (ISO/IEC 23008-1
  §10.3.3), possibly with an associated MP table.

Required on the MMTP session carrying an associated Asset (same `packet_id` as that Asset):

- **Hypothetical Receiver Buffer Model message** (ISO/IEC 23008-1 §10.4.2).
- **Hypothetical Receiver Buffer Model Removal message** (ISO/IEC 23008-1 §10.4.9).

None of the above four ISO-defined message/table bodies are transcribed here (see the scope
decision above).

## 2. `mmt_atsc3_message()` — the ATSC-original MMTP signaling envelope (§7.2.3.1)

This is the one MMT-path structure that is entirely ATSC's own (`message_id` is in the
"user private" range of ISO/IEC 23008-1 §10.7, but A/331 fully specifies its payload), so it is
fully transcribed:

### Table 7.9 — Bit Stream Syntax for `mmt_atsc3_message()`
_§7.2.3.1, A/331:2025-06 p.88_

| Syntax | No. of Bits | Format |
|---|---|---|
| `mmt_atsc3_message() {` |
| `message_id` | 16 | uimsbf |
| `version` | 8 | uimsbf |
| `length` | 32 | uimsbf |
| `message payload {` |
| `service_id` | 16 | uimsbf |
| `atsc3_message_content_type` | 16 | uimsbf |
| `atsc3_message_content_version` | 8 | uimsbf |
| `atsc3_message_content_compression` | 8 | uimsbf |
| `URI_length` | 8 | uimsbf |
| `for (i=0; i<URI_length; i++) {` |
| `URI_byte` | 8 | uimsbf |
| `}` |
| `atsc3_message_content_length` | 32 | uimsbf |
| `for (i=0; i<atsc3_message_content_length; i++) {` |
| `atsc3_message_content_byte` | 8 | uimsbf |
| `}` |
| `for (i=0; i<length-11-URI_length-atsc3_message_content_length) {` |
| `reserved` | 8 | uimsbf |
| `}` |
| `}` |
| `}` |

- **`message_id`** — fixed value **`0x8100`** identifying this as an `mmt_atsc3_message()`.
- **`version`** — increments by 1 (wraps `255 -> 0`) on any content change.
- **`length`** — length in bytes of the message from the byte after this field to the message's
  last byte.
- **`service_id`** — associates the payload with `SLT.Service@serviceId`.
- **`atsc3_message_content_type`** — identifies the payload's content type; Table 7.10 below.
- **`atsc3_message_content_version`** — increments by 1 (wraps at max) whenever the content
  identified by the `(service_id, atsc3_message_content_type, URI)` tuple changes.
- **`atsc3_message_content_compression`** — Table 7.11 below.
- **`URI_length`** / **`URI_byte`** — UTF-8 URI (no terminating null) uniquely identifying the
  payload across Services; `URI_length=0` = absent. **Required** (non-zero) when
  `atsc3_message_content_type = 0x0002` (MPD).
- **`atsc3_message_content_length`** / **`atsc3_message_content_byte`** — the payload content
  itself (interpretation per `atsc3_message_content_type`).

### Table 7.10 — Code Values for `atsc3_message_content_type`
_§7.2.3.1, A/331:2025-06 p.89_

| Value | Meaning |
|---|---|
| 0x0000 | ATSC Reserved |
| 0x0001 | `UserServiceDescription` (Table 7.8, §3 below) |
| 0x0002 | MPD (DASH-IF, external, not transcribed) |
| 0x0003 | HELD (see [`a331-signalling.md`](a331-signalling.md) §4.5) |
| 0x0004 | Application Event Information (A/337, external, not transcribed) |
| 0x0005 | Video Stream Properties Descriptor (§7.2.3.2 — not transcribed, see §4) |
| 0x0006 | ATSC Staggercast Descriptor (§7.2.3.3 — not transcribed, see §4) |
| 0x0007 | Inband Event Descriptor (A/337, external, not transcribed) |
| 0x0008 | Caption Asset Descriptor (§7.2.3.5 — not transcribed, see §4) |
| 0x0009 | Audio Stream Properties Descriptor (§7.2.3.4 — not transcribed, see §4) |
| 0x000A | DWD (see [`a331-signalling.md`](a331-signalling.md) §4.6) |
| 0x000B | RSAT (A/200, external, not transcribed) |
| 0x000C | Security Properties Descriptor (§7.2.4.2.1.1 — not transcribed, see §4) |
| 0x000D-0xFFFF | Industry Reserved (ATSC Code Point Registry) |

### Table 7.11 — Code Values for `atsc3_message_content_compression`
_§7.2.3.1, A/331:2025-06 p.89_

| Value | Meaning |
|---|---|
| 0x00 | ATSC Reserved |
| 0x01 | No compression |
| 0x02 | gzip (RFC 1952) |
| 0x03 | (deprecated) |
| 0x04-0xFF | ATSC Reserved |

## 3. User Service Description for MMTP (USBD)

Root element `BundleDescriptionMMT`, namespace
`tag:atsc.org,2016:XMLSchemas/ATSC3/Delivery/MMTUSD/1.0/`, schema
`MMTUSD-1.0-20210401.xsd`. Media type `application/mmt-usd+xml` (Annex H.3, file ext `.musd`).
Modeled on the same 3GPP MBMS USBD base as the ROUTE USBD (§4.1 of
[`a331-signalling.md`](a331-signalling.md)), with MMTP-specific extensions.

### Table 7.8 — XML Format of the User Service Bundle Description Fragment for MMTP
_§7.2.1, A/331:2025-06 p.80-82_

| Element / Attribute | Use | Data Type | Description |
|---|---|---|---|
| `BundleDescriptionMMT` | | | Root element. |
| `UserServiceDescription` | 1 | | One ATSC 3.0 Service. |
| `@serviceId` | 1 | unsignedShort | Matches the referencing `SLT.Service@serviceId`. |
| `@serviceStatus` | 0..1 | boolean | Active/inactive. Default `true`. |
| `Name` / `@lang` | 0..N / 1 | string / lang | Service name, per language. Same shape as the ROUTE USBD. |
| `ServiceLanguage` | 0..N | lang | Deprecated — backwards-compat only. |
| `ContentAdvisoryRating` | required unless unrated | `sa:CARatingType` | RRT-based content advisory rating (A/332). Empty content when rated "exempt". |
| `OtherRatings` | 0..N | `sa:OtherRatingType` | Non-RRT content advisory rating(s); each instance needs a unique `@ratingScheme`. |
| `Channel` | 1 | | Service display info. |
| `Channel@serviceGenre` | 0..1 | unsignedByte | Primary genre; term ID from A/153 Part 4 Annex B classification scheme. |
| `Channel@serviceIcon` | 1 | anyURI | Service icon URL. |
| `Channel.ServiceDescription` | 0..N | | Service description text, per language. |
| `ServiceDescription@serviceDescrText` | 1 | string | Description text. |
| `ServiceDescription@serviceDescrLang` | 0..1 | lang | Description language; default `"en"`. |
| `MPUComponent` | 0..1 | | MPU-delivered Component info. |
| `MPUComponent@mmtPackageId` | 1 | string | Reference to the current MMT Package. |
| `@contentIdSchemeIdUri` / `@contentIdValue` | 0..1 each | anyURI / string | Content ID scheme + value for the current MMT Package (`urn:eidr` or SMPTE 2092-1 Designator, or a private scheme). Both present or both absent. |
| `@nextMMTPackageId` | 0..1 | string | Reference to the MMT Package used after the current one. |
| `@nextContentIdSchemeIdUri` / `@nextContentIdValue` | 0..1 each | anyURI / string | Same content-ID pairing for the *next* MMT Package. |
| `ROUTEComponent` | 0..1 | | Locally-cached content delivered over ROUTE alongside an MMTP Service. |
| `ROUTEComponent@sTSIDUri` | 1 | anyURI | Reference to the S-TSID fragment for this ROUTE-delivered content (see [`a331-signalling.md`](a331-signalling.md) §4.2 / [`a331-route.md`](a331-route.md)). |
| `@apdUri` | 0..1 | anyURI | Reference to the APD fragment (file repair). If present, the referenced S-TSID's `EFDT` must carry >=1 `Alternate-Content-Location-1` element. |
| `@sTSIDDestinationIpAddress` / `@sTSIDDestinationUdpPort` / `@sTSIDSourceIpAddress` | 0..1/1/1 | IPv4/unsignedShort/IPv4 | Transport address of the S-TSID-carrying packets. Destination IP defaults to the current MMTP session's destination IP when absent. |
| `@sTSIDMajorProtocolVersion` / `@sTSIDMinorProtocolVersion` | 0..1 each | unsignedByte | Default `1` / `0`. |
| `BroadbandComponent` | 0..1 | | Broadband-delivered Component info. At least one of `MPUComponent`/`ROUTEComponent`/`BroadbandComponent` shall be present. |
| `BroadbandComponent@fullMPDUri` | 1 | anyURI | Reference to the MPD for broadband-delivered Components. |
| `BroadbandComponentInfo` | 0..N | | Cross-references a broadband DASH Representation against broadcast MMT Asset(s); required when such a dependency or content-identity relationship exists. |
| `@repId` | 1 | StringNoWhitespace | The broadband DASH Representation's ID. |
| `@complementaryAssetId` / `@dependentAssetId` | 0..1 each | space-separated string list | MMT Asset IDs the Representation depends on / that depend on the Representation (e.g. base-layer/enhancement-layer scalable coding). |
| `@simulcastAssetId` | 0..1 | string | MMT Asset ID carrying the same content as this Representation. |
| `ComponentInfo` | 0..N | | Per-Component metadata for MPU-delivered Components; required when `MPUComponent` is present. |
| `@componentType` | 1 | unsignedByte | `0`=audio, `1`=video, `2`=closed caption, `3`-`7`=ATSC Reserved. |
| `@componentRole` | 1 | unsignedByte | Meaning depends on `@componentType` — see below. |
| `@componentProtectedFlag` | 0..1 | boolean | Whether this Component is encrypted. Default `false`. |
| `@componentId` | 1 | string (URI or RFC 4122 UUID, `urn:uuid:` prefix omitted) | Must equal the MP table's `asset_id` for this Component. |
| `@componentName` | 0..1 | string | Human-readable Component name. |

`@componentRole` code values (§7.2.1.1):
- Audio (`componentType=0`): `0`=Complete main, `1`=Music and Effects, `2`=Dialog,
  `3`=Commentary, `4`=Visually Impaired, `5`=Hearing Impaired, `6`=Voice-Over,
  `7`-`254`=ATSC Reserved, `255`=unknown.
- Video (`componentType=1`): `0`=Primary video, `1`-`254`=ATSC Reserved, `255`=unknown.
- Closed Caption (`componentType=2`): `0`=Normal, `1`=Easy reader, `2`-`254`=ATSC Reserved,
  `255`=unknown.
- When `componentType` is `3`-`7`, `@componentRole` shall be `255`.

## 4. Not transcribed — catalog of what remains, with clause numbers

These exist in A/331 §7.2.3-§7.2.4, carried as `atsc3_message_content` payloads inside
`mmt_atsc3_message()` (see Table 7.10 above for the content-type codes that select them). Each
is its own multi-table bit-syntax section; none are transcribed in this pass:

| Descriptor | Clause | Content type |
|---|---|---|
| Video Stream Properties Descriptor (incl. scalability/multi-view/resolution/picture-rate/bit-rate/color/transfer-function sub-tables — Tables 7.12-7.21) | §7.2.3.2 | 0x0005 |
| ATSC Staggercast Descriptor | §7.2.3.3 (Table 7.22) | 0x0006 |
| Audio Stream Properties Descriptor (incl. AC-4 `profile_level_indication()`, MPEG-H `audio_channel_config()`, rendering/accessibility/role sub-tables — Tables 7.23-7.28) | §7.2.3.4 | 0x0009 |
| Emergency Information Time Information | §7.2.3.4.2 (Table 7.29) | (sub-element of the audio descriptor) |
| Multi-Stream Information | §7.2.3.4.3 (Table 7.30) | (sub-element) |
| Presentation Aux-Stream Information | §7.2.3.4.4 (Table 7.31) | (sub-element) |
| Caption Asset Descriptor | §7.2.3.5 (Tables 7.32-7.34) | 0x0008 |
| Security Properties Descriptor (CENC scheme signalling, `default_KID`, license acquisition URL/type) | §7.2.4.2.1.1 (Tables 7.35-7.36) | 0x000C |
| Low Delay Decryption Information Descriptor | §7.2.4.2.1.2 (Tables 7.37-7.38) | (carried alongside security properties) |
| MMT Hint Sample for Low Delay Decryption (sample description/format, low-delay-decryption-info structures) | §7.2.4.2.2 | (MPU-internal structure, not an `mmt_atsc3_message` content type) |
| MPU Fragment Type for MMT Hint Sample | §7.2.4.2.3 (Table 7.39) | (MPU-internal structure) |
| SIGNED_MMT_MESSAGE | §7.2.3 area (referenced, not detailed in this pass) | — |
| Content Advisory Ratings in MMTP signaling (RRT-based and non-RRT) | §7.3 (partially covered above via `ContentAdvisoryRating`/`OtherRatings`) | — |

### DRM/CENC overview (§7.2.4.1, informative summary only — not the descriptor bit-tables)

- Content protection in MMT relies on **ISO/IEC 23001-7** (Common Encryption in ISOBMFF, CENC)
  — external, not vendored; `transmux` already implements CENC for the ISOBMFF/CMAF path (see
  "Overlap" below).
  Protection info flows two ways: via MMT signaling messages (the Security Properties
  Descriptor above), and via metadata boxes inside the MPU itself (ISOBMFF `pssh`, etc.).
- Service-level encryption is flagged via `SLT.Service@protected` / `@drmSystemID` (see
  [`a331-signalling.md`](a331-signalling.md) §3) and, per-Component, via
  `ComponentInfo@componentProtectedFlag` in the MMTP USBD above.
- The `pssh` box's content (when present in `moov`/`moof`) must match the `pssh` data carried in
  the Security Properties Descriptor; per DASH-IF IOP v5 Part 6 §6.3, Initialization Segments
  should carry **no** `pssh` boxes at all (receivers may ignore any found there) — this mirrors
  the identical rule already noted for ROUTE/DASH in
  [`a331-signalling.md`](a331-signalling.md) §3 (`Service@drmSystemID` semantics).
- Out-of-order delivery (media data delivered before `moof`/`moov`) is supported via the Low
  Delay Decryption Information Descriptor, keyed by MPU sequence number / movie fragment
  sequence number / sample number.

## Overlap with other workspace crates

- **ISO/IEC 23008-1 (MMT base)** is not vendored and not freely available — this is the single
  biggest gap for MMT support in this workspace; no crate here currently implements MP-table/MPU
  framing. If MMT is prioritized in the future, acquiring or licensing that standard is a
  prerequisite, independent of anything ATSC-specific.
- **`transmux`** already implements CENC (`ISO/IEC 23001-7`) for the ISOBMFF/CMAF path; the MMT
  DRM signalling above (Security Properties Descriptor) should map onto `transmux`'s existing
  CENC types rather than growing a parallel CENC model inside `atsc3`.
- **ROUTE's `HELD`/`DWD` fragments are shared verbatim** with the MMT path (§1 above) — a single
  implementation in the new `atsc3` crate, not duplicated per-transport-path code.
