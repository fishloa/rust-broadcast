# DVB-MABR multicast transport formats — ETSI TS 103 769 V1.2.1 clauses 8, 9, 12, Annexes F, H

Source: ETSI TS 103 769 V1.2.1 (2024-11). See [`README.md`](README.md) for provenance.

This is the field-by-field reference for the wire formats an implementer of the crate's
transport layer parses/builds. **Read the important scope caveat in §0 first**: TS 103
769 itself defines almost none of the packet-header bits directly — it *profiles* other
specifications, most of which are already implemented elsewhere in this workspace or are
the subject of a separate open issue.

## 0. Scope: what TS 103 769 actually defines at the wire level

TS 103 769 has exactly two **normative transport annexes**, and both work by reference,
not by original bit-layout definition:

| Annex | Protocol profiled | Base spec (not TS 103 769) | Already in this workspace? |
|---|---|---|---|
| **F** (clause F, normative) | 3GPP FLUTE (the MBMS Download Profile) | 3GPP TS 26.346 clause L.4 (referencing IETF RFC 3926 FLUTE, RFC 5651 LCT, RFC 5775 ALC) | **Yes** — `rmt-flute` (published, v0.4.0) already implements the RFC 5651 `LctHeader`, the RFC 5775 `AlcPacket` + `EXT_FTI`, and the RFC 6726 FLUTE `EXT_FDT`/`EXT_CENC` extensions. TS 103 769 Annex F only adds an **XML** extension to the FDT-Instance body (§F.2.3-F.2.5, covered below) and a set of URI/operational conventions layered on top — it changes no LCT/ALC/FLUTE bit ever emitted on the wire. |
| **H** (clause H, normative) | ROUTE, per ATSC A/331 Annex A | ATSC A/331:2019 Annex A (which itself profiles RFC 5651 LCT / RFC 5775 ALC, plus ATSC-specific LCT header extensions `EXT_TOL` and `EXT_TIME`, S-TSID/MPD signalling and the ROUTE `Codepoint` registry) | **No.** `rmt-flute` does not implement `EXT_TOL`/`EXT_TIME`/ROUTE Codepoints or S-TSID — that byte-level and XML-signalling work belongs to **issue #750** ("ATSC 3.0: A/331 ROUTE + MMT signalling, A/321 bootstrap"), which is open and unimplemented as of this writing. **`dvb-mabr` cannot fully implement Annex H until #750 (or an equivalent ROUTE-primitives crate) exists**; this document records what TS 103 769 constrains on top of ROUTE, not the ROUTE packet layout itself. |

**Recommendation for implementation planning**: `dvb-mabr` should depend on `rmt-flute`
for the FLUTE/LCT/ALC packet layer (Annex F requires nothing further at that layer), and
should either depend on the crate that comes out of issue #750 for the ROUTE layer, or
treat Annex H support as blocked on that issue. Do not re-implement LCT/ALC/FLUTE header
parsing inside `dvb-mabr` — that would duplicate `rmt-flute` and drift from it.

The **byte-exact LCT/ALC/FLUTE header layout is out of scope of this document** — see
`rmt-flute/docs/lct.md`, `alc.md`, `flute.md` in this workspace for that transcription
(RFC 5651/5775/6726, already spec-cited there). Likewise the exact ROUTE/LCT-extension
bit layout (`EXT_TOL`, `EXT_TIME`, Codepoint semantics) is **not established here** — it
lives in ATSC A/331 Annex A, which is not yet vendored or transcribed anywhere in this
workspace (see [`README.md`](README.md) "could not establish" list).

What follows is everything TS 103 769 itself specifies: the multicast-transport-object
concept common to both protocols (clause 8), the FLUTE-specific XML/URI profiling
(Annex F), the ROUTE-specific profiling *above* the packet layer (Annex H, excluding the
packet bytes themselves), unicast repair (clause 9), and the integrity/authenticity
metadata profiles (clause 12) both annexes build on.

## 1. Multicast transport objects (clause 8.3.4, protocol-agnostic)

A **multicast transport object** is the unit carried at reference point `M`. Every
ingest object (a DASH/HLS segment, init segment, or presentation manifest) maps to one
or more multicast transport objects under one of two **multicast transmission modes**
(clause 8.3.4.1), signalled per multicast transport session (see
[`mabr-signalling.md`](mabr-signalling.md) §`@transmissionMode`):

- **Resource transmission mode** (`resource`) — exactly one ingest object -> one
  multicast transport object.
- **Chunked transmission mode** (`chunked`) — one ingest object -> one or more multicast
  transport objects (e.g. one per HTTP chunk / CMAF chunk), enabling low-latency
  delivery. The Multicast server signals end-of-object either by a zero-length final
  transport object or a query component (`isLast`, see §2.2 below); not all chunks carry
  a Random Access Point, so a `hasRandomAccessPoint` marker may be added per object.

Common rules (normative, clause 8.3.4):
- Emitted only while the multicast transport session is in the **active** state (clause
  10.3.0).
- Must not exceed the session's configured maximum bit rate (clause 10.2.3.10).
- FEC, if used, may be addressed to the same or a different multicast group/port than
  the objects it protects (clause 8.3.4.2); the endpoint address is signalled per clause
  10.2.3.11.
- Integrity/authenticity metadata is added per clause 8.3.4.5/8.3.4.6 when the session's
  `@transportSecurity` value requires it (`integrity` or `integrityAndAuthenticity`; see
  §4 below).
- An **in-band ancillary object carousel** (clause 8.3.4.4) may repeat presentation
  manifests, init segments, and other resources on the same or a dedicated "multicast
  gateway configuration transport session" (clause 8.3.5) so a gateway can bootstrap
  without ever using reference point `A`.

## 2. Annex F — FLUTE profile (normative)

Base: 3GPP MBMS Download Profile (3GPP TS 26.346 clause L.4) + MBMS DASH Streaming
(clause 5.6 of the same). TS 103 769 differences from the base 3GPP profile (clause F.0):
MBMS User Service Discovery/Announcement (SDP) is replaced by the multicast session
configuration document; RTSP session setup is not required; byte-range File Repair
(3GPP clause 9.3.6.2) is permitted but `Alternate-Content-Location-1/2` FDT elements and
the Associated Delivery Procedure Description are **ignored**; symbol-based repair is
**not supported** (no Associated Delivery Procedure Description); a chunked-transmission
convention is added (§2.2); the Close Object (`B`) LCT flag may be used in low-loss
deployments, set only on the session's final packet for an object.

### 2.1 Session mapping (clause F.2.0)

- One multicast transport session = one FLUTE Session comprising one FLUTE Channel (LCT
  channel), per 3GPP TS 26.346 clause L.4.
- One multicast transport object = one 3GPP FLUTE Transport Object.
- The transport object URI is `File/@Content-Location`; its size is `File/@Content-Length`.
- The FDT Instance describing an object is always carried as TOI=0 in the same FLUTE
  Session.
- Signalling in the multicast session configuration (clause F.1):
  `TransportProtocol/@protocolIdentifier` =
  `urn:dvb:metadata:cs:MulticastTransportProtocolCS:2019:FLUTE`,
  `@protocolVersion` = `1`. AL-FEC is always in-band (same FLUTE Session); omitting
  `ForwardErrorCorrectionParameters` means the **Compact No-Code** AL-FEC scheme (IETF
  RFC 5445 §3) is in use.

### 2.2 Chunked transmission mode URI convention (clause F.2.2, normative)

In chunked mode, `File/@Content-Location` is suffixed with a **fragment identifier**
(RFC 3986 §3.5) giving the byte offset of this transport object's first byte within the
overall media object (the "chunk offset"), and may carry these **query components**
(RFC 3986 §3.5):

| Query component | Meaning |
|---|---|
| `isLast` | This transport object is the final chunk of the media object. |
| `hasRandomAccessPoint` | This transport object is a valid point to start decoding. |

Example progression for one media object (from the spec, clause F.2.2):

```
tag:multicastserver.isp.net,2019-01-01:mts100,3922:segment1234.m4s?hasRandomAccessPoint#0
tag:multicastserver.isp.net,2019-01-01:mts100,3922:segment1234.m4s#5321
...
tag:multicastserver.isp.net,2019-01-01:mts100,3922:segment1234.m4s?isLast&hasRandomAccessPoint#154360
```

All transport objects for one media object share the same URI prefix (hier-part per RFC
3986). A gateway starts assembling a new playback delivery object on receiving offset 0,
and serves it at `L` with HTTP chunked transfer coding until end-of-object is detected
(zero-length final object, or the `isLast` component).

### 2.3 Extended FDT — integrity (`Content-Digest`/`Repr-Digest`) (clause F.2.3, normative)

3GPP FLUTE's own `File/@Content-MD5` (3GPP TS 26.346 clause L.4.4) is deprecated here
because MD5 is collision-vulnerable and therefore unsafe to combine with the
authenticity check below. TS 103 769 extends the FDT schema with two new attributes,
each an HTTP-Digest-Fields-style string (see §4.1):

| Attribute | Use | Meaning |
|---|---|---|
| `File/@Content-Digest` | 0..1 | Digest of **this transport object's payload**. |
| `File/@Repr-Digest` | 0..1 | Digest of the **whole media object** (only present in the FDT for the final chunk, chunked mode). |

Value syntax: `<hash-alg>=:<base64 digest>:` (identical to the HTTP `Content-Digest`
field, IETF RFC 9530 §2).

### 2.4 Extended FDT — authenticity (`File/Signature`) (clause F.2.4, normative)

3GPP FLUTE has no authenticity mechanism; TS 103 769 adds a `File/Signature` child
element (0..n), modelled on IETF RFC 9421 (HTTP Message Signatures) but simplified to
one signature per element instance (no structured-field dictionary, no `tag` parameter).

**Table F.2.4.1-1 : `Signature` element syntax**

| Element/attribute | Use | Data type | Description |
|---|---|---|---|
| `File/Signature` | 0..n | String (Base64, RFC 4648) | Cryptographic signature over the metadata components named in `@scope`. |
| `@scope` | 1 | String | Ordered, comma-separated list of protected components, each an XPath relative to the parent `File` element prefixed with `@` (e.g. `@@TOI` for the `File/@TOI` attribute, `@Cache-Control` for a child element, `@Example/Element/@attribute` for a nested attribute). |
| `@created` | 0..1 | Integer | Signature creation time, UNIX timestamp. |
| `@expires` | 0..1 | Integer | Signature expiry time, UNIX timestamp. |
| `@algorithm` | 1 | String | Message signature algorithm, from the HTTP Message Signature Algorithm Registry. |
| `@keyUri` (prose) / `keyId` (XSD) | 1 | URI string (prose) / `xs:hexBinary` (XSD) | ⚠️ **SPEC CONFLICT** — see below. |

**⚠️ Flagged spec-internal conflict — `@keyUri` vs `keyId`:**

The prose Table F.2.4.1-1 (clause F.2.4.1, page 137) names this attribute
**`@keyUri`** and types it as a **URI string**. The Annex F.2.5.1 XSD
(page 138), which is the normative authority for XML validation, names it
**`keyId`** and types it as **`xs:hexBinary`**.

These disagree on both *name* and *type*:
- Prose table: `@keyUri`, URI string — suggests a dereferenceable key reference.
- XSD: `keyId`, `xs:hexBinary` — the raw X.509 Subject Key Identifier bytes.

An implementation that validates against the XSD will reject `@keyUri`;
one that reads the prose will look for the wrong attribute name. Until an
erratum resolves this, an implementation should follow the XSD (as the
normative wire-format authority) or accept both attribute names.

Normative constraints: when used to assert transport-object authenticity, `@scope` must
include `@@Content-Digest`; the signature base is built per RFC 9421 §2.5 in the exact
order `@scope` lists; multiple `Signature` elements (one per algorithm) are permitted,
each with a distinct `@algorithm`.

### 2.5 Extended FDT XML Schema (clause F.2.5.1, normative, reproduced in full)

Namespace `urn:dvb:metadata:ExtendedFileDeliveryTable:2022`, importing the 3GPP FDT
namespace `urn:3GPP:metadata:2022:FLUTE:FDT`. Defines `SignatureType` (the
`File/Signature` element of §2.4), `MessageDigestType` (pattern
`[a-z][-_.*a-z0-9]*=:[a-zA-Z0-9+/=]*:`), and extends the 3GPP `FileType` with the
optional `Content-Digest`/`Repr-Digest` attributes (both typed `MessageDigestType`) and
an unbounded `Signature` child sequence. ⚠️ **Note:** the XSD names the signature
key attribute **`keyId`** (typed `xs:hexBinary`), not `@keyUri` as the prose table
in §2.4 does — see the flagged conflict in §2.4 above.

### 2.6 Multicast gateway operation, FLUTE (clause F.3)

- Gateways support reception of all encoding symbols per 3GPP TS 26.346 clause L.4.7;
  repair symbols may be ignored if FEC is not implemented.
- Unicast repair (clause 9) starts at latest at FDT-Instance expiry (3GPP clause 9.3.2)
  or on receiving a FLUTE packet with the Close Object (`B`) flag set.
- `Alternate-Content-Location-1/2` FDT elements are **ignored**.
- Resource-mode byte range: per 3GPP clause 9.3.6.2. Chunked mode: add the chunk offset
  (§2.2) to the byte range computed the same way — e.g. a missing packet from a chunk at
  offset 5321 starts its repair range at `5321 + ESI x SymbolSize`.

## 3. Annex H — ROUTE profile (normative)

Base: ROUTE as defined in ATSC A/331 Annex A. TS 103 769 differences from A/331 (clause
H.0): only Codepoints 5-9 are used (clause H.4); MPD-less start-up mode is **prohibited**
(clause H.5.1); `Alternate-Content-Location-1/2` Extended FDT elements are **ignored**
(clause H.6.2); an exception to basic delivery-object-recovery timing is specified
(clause H.8.0).

**The ROUTE/LCT packet header bit layout itself (the figure in clause H.2.0 showing UDP
header + Default LCT header + FEC Payload ID + payload) is defined by ATSC A/331 Annex
A / RFC 5651 / RFC 5775, not by TS 103 769, and is not transcribed here** — see §0 above.
What follows is only what TS 103 769 additionally constrains.

### 3.1 Signalling in the multicast session configuration (clause H.1)

`TransportProtocol/@protocolIdentifier` =
`urn:dvb:metadata:cs:MulticastTransportProtocolCS:2019:ROUTE`, `@protocolVersion` = `1`.
The LCT Transport Session Identifier is conveyed in
`EndpointAddress/MediaTransportSessionIdentifier`. Omitting
`ForwardErrorCorrectionParameters` means no ROUTE Repair Flow protects the session.

### 3.2 LCT header field constraints for CMAF chunks (Table H.2.0-1, normative)

| LCT header field | ROUTE Source Flow, plain DASH segments | ROUTE Source Flow, CMAF-chunked segments | ROUTE Repair Flow |
|---|---|---|---|
| Close Object (`B`) flag | Per ATSC A/331 clause 7.1.7. | Set iff the packet contains the last byte of the transport object. | Per RFC 5651. |
| Congestion Control Information (CCI) | — | Conveys the earliest presentation time in the packet; 32 bits (`C`=0) or 64 bits (`C`=1). | Not applicable. |
| FEC Payload ID | Conveys `start_offset` of the LCT Payload Data field (ATSC A/331 clause A.3.5). | Same. | Not applicable. |

LCT header extensions used (clause H.2.1): `EXT_TOL` (24 or, if needed, 48-bit Transfer
Length — used when the Close-Object-flagged packet may be lost) and `EXT_TIME` (Sender
Current Time, per ATSC A/331 clause 8.1.1). **The exact bit layout of `EXT_TOL`/
`EXT_TIME` is defined in ATSC A/331, not established here** (see README).

### 3.3 Delivery object mode (clause H.3, normative)

Signalled by `Payload/@formatId` in the Source Flow session metadata: `1` = **File
Mode**, `2` = **Entity Mode**. Only these two values are permitted.

- **File Mode** (clause H.3.1): object metadata rides in the Extended FDT, embedded in
  the session/object signalling (S-TSID, §3.4). Per-object-varying fields
  (`Content-Location`, `Content-Length`) are **not** carried there — `Content-Location`
  is derived locally from a `$TOI$` file template (`FDT-Instance/@fileTemplate`); length
  comes from the `EXT_TOL` LCT extension.
- **Entity Mode** (clause H.3.2): object metadata is expressed as HTTP/1.1 entity header
  fields (RFC 9110 §8) sent alongside the object.
  - **Resource transmission mode** (H.3.2.1): object sent as `HTTP header (with
    Content-Length) + concatenated CMAF chunks`, no chunked transfer coding. Integrity
    via `Content-Digest`/`Repr-Digest` (not MD5); authenticity via a `Signature` field —
    both may ride as HTTP trailer fields (RFC 9110 §6.5).
  - **Chunked transmission mode** (H.3.2.2): object sent as an HTTP/1.1 chunked-transfer
    message (RFC 9112 §7.1); a CMAF chunk need not map 1:1 to an HTTP chunk. Integrity
    per-chunk via the `chunk-content-digest` HTTP/1.1 chunk extension (clause 12.1.2,
    same syntax as FLUTE's chunk digest, §4.2 below), whole-object via a `Repr-Digest`
    trailer; authenticity per-chunk via `chunk-signature` (clause 12.2.2, §4.2), whole-
    object via a `Signature` trailer.

### 3.4 In-band session metadata: S-TSID mapping (clause H.5, normative)

Session metadata (S-TSID / MPD fragments, per ATSC A/331 clause 7.1 service-level
signalling) is carried in-band at `M` in a dedicated LCT channel `TSI=0` of the same
multicast transport session it describes.

**Table H.5.0-1 : Multicast transport session -> S-TSID `RS` element mapping**

| `MulticastTransportSession` parameter (clause 10.2.3) | S-TSID element/attribute |
|---|---|
| `EndpointAddress/NetworkSourceAddress` | `RS/@sIpAddr` |
| `EndpointAddress/NetworkDestinationGroupAddress` | `RS/@dIpAddr` |
| `EndpointAddress/TransportDestinationPort` | `RS/@dPort` |
| `EndpointAddress/MediaTransportSessionIdentifier` | `RS/LS/@tsi` |
| `@start` | `RS/LS/@startTime` |
| `@start` + `@duration` | `RS/LS/@endTime` |
| `BitRate/@maximum` | `RS/LS/@bw` |

A single ROUTE Session (one destination IP+port) may carry several
`MulticastTransportSession`s distinguished only by `LS/@tsi`. When a Repair Flow shares
the Source Flow's ROUTE Session (same group/port, different `tsi`), its FEC parameters
map per **Table H.5.0-2** (`OverheadPercentage` -> `RS/LS/RepairFlow/FECParameters/@overhead`,
its own `MediaTransportSessionIdentifier` -> `RS/LS/@tsi`); when the Repair Flow uses a
*different* ROUTE Session, an independent `RS` element is used per **Table H.5.0-3**
(mapping the FEC parameters' own `EndpointAddress` fields to `RS/@sIpAddr`/`@dIpAddr`/
`@dPort` plus `RS/LS/@tsi` and the overhead attribute, as above).

`SrcFlow`/`RepairFlow` elements describe each flow per ATSC A/331 clauses A.3/A.4.3;
`MediaInfo/@startUp` is fixed `false` (MPD-less start-up is prohibited by this profile).

### 3.5 Codepoint signalling (clause H.4, normative)

A Codepoint value from **5 through 9 exclusively** shall be used in the LCT header:
DASH initialization segments use 5, 6, or 7; DASH media segments use 8 (File Mode) or 9
(Entity Mode). The full Codepoint registry semantics are in ATSC A/331 Table A.3.6 — not
reproduced here (out of scope, see §0).

### 3.6 Multicast gateway/server operation, ROUTE (clauses H.7-H.8)

- Both transmission modes map one ingest object to one transport object (H.7) — unlike
  FLUTE, ROUTE's "chunked" and "resource" modes differ only in HTTP chunked-transfer
  framing, not in transport-object cardinality.
- FEC follows ATSC A/331 Annex A.4, signalled via `S-TSID/RS/LS/RepairFlow` (clause
  H.8.1).
- Unicast repair (clause H.8.2) follows clause 9 (below), but ATSC A/331's file-repair
  procedure (clause 7.1.7.2, using the Close Object/Close Session flags) may trigger it
  earlier. The missing byte range between a received packet `i` and the next received
  packet `k` is `start_offset(i) + size(i)` .. `start_offset(k) - 1`.
- Exception to basic recovery timing (clause H.8.0): if `MPD/@availabilityTimeOffset` is
  signalled, the gateway must enable HTTP/1.1 chunked transfer coding at `L` rather than
  waiting for full packet-set recovery before serving (per ATSC A/331 clause A.3.10.2).

### 3.7 Implementation guidance (Annex I, informative — not normative, recorded for context)

- One Representation per LCT session/TSI; `$Number$` templating -> File Mode,
  `$Time$`/other addressing -> Entity Mode (clause I.1.2).
- `start_offset` in the FEC Payload ID = byte offset within the media object; the LSB of
  the LCT PSI field may flag "first packet of a CMAF Random Access chunk" for fast
  tune-in (clauses I.1.3.1, I.2.2); `EXT_TOL` is added once total length is known.
  CCI carries the earliest presentation time of the CMAF chunk when that PSI bit is set
  (clause I.1.4).

## 4. Security profiles common to both annexes (clause 12, normative)

Both Annex F (§2.3-2.4) and Annex H (§3.3) build their integrity/authenticity metadata
on these two profiles.

### 4.1 Integrity: HTTP Digest Fields profile (clause 12.1)

Profiles IETF RFC 9530 for unidirectional multicast:
- Implementations support at least `sha-256` and `sha-512` (IANA Hash Algorithms for
  HTTP Digest Fields registry).
- `Want-Repr-Digest`/`Want-Content-Digest` are **not used** (the gateway never makes an
  explicit per-object request).
- `Content-Digest` covers the transport object's payload (post any `Content-Encoding`);
  may ride as an HTTP trailer field (RFC 9110 §6.5).
- `Repr-Digest` (optional) covers the *whole* media object when the transport object is
  only a partial byte range or differently encoded — set on the transport object
  carrying the final byte range.
- Insecure digest algorithms (per RFC 9530 §5 criteria) must not be combined with
  authenticity assertion (clause 12.1.3).

**Chunk-level digest** (clause 12.1.2) — `chunk-content-digest` HTTP/1.1 chunk
extension, ABNF:

```
chunk-content-digest = chunk-name "=" digest
chunk-name            = "chunk-content-digest"
digest                = DQUOTE hash-key "=:" digest-value ":" DQUOTE
hash-key              = string
string                = 1*(ALPHA / DIGIT / "-")
digest-value          = base64-str
base64-str            = 1*(ALPHA / DIGIT / "+" / "/") *("=")
```

### 4.2 Authenticity: HTTP Message Signatures profile (clause 12.2)

Profiles IETF RFC 9421 for unidirectional multicast (no request/response binding, since
there is no explicit per-object request):
- **Derived components** (clause 12.2.1.1): `@method`/`@status` are **not used**
  (no HTTP request/response exists to derive them from). `@target-uri` is redefined as
  the *complete multicast transport object URI including any fragment*. `@scheme`,
  `@authority`, `@request-target`, `@path`, `@query`, `@query-param` apply to that URI.
  A **new** derived component `@fragment` is introduced: the transport object URI's
  fragment identifier (RFC 3986 §3.5) excluding the leading `#` (example: URI
  `tag:...:segment1234.m4s#2` -> signature base line `"@fragment": #2`).
- **Minimum protected components** (clause 12.2.1.2): signature parameters + a
  standard-hash-algorithm content digest (clause 12.1.1); should-protect: the transport
  object URI and any per-object identifier (e.g. TOI).
- **Signature parameters** (clause 12.2.1.3): `alg` (required), `keyid` (required) =
  hex-encoded X.509 subject key identifier (clause 12.2.1.6).
- **Algorithm support floor** (clause 12.2.1.5): asymmetric — at least
  `RSASSA-PKCS1-v1_5 using SHA-256`; symmetric — at least `HMAC using SHA-256`
  (symmetric use flagged as a security risk in clause 12.2.3: a compromised shared
  secret lets an attacker impersonate the sender).
- **Certificate verification** (clause 12.2.1.6): asymmetric keys are X.509 v3 with a
  Subject Key Identifier extension (RFC 5280 §4.2.1.2); CN or a SAN must match either
  `NetworkSourceAddress` or the transport-object-URI hostname; certificates delivered
  over `M` must themselves be path-validated (RFC 5280 §6) before use.

**Chunk-level signature** (clause 12.2.2) — `Chunked-Signature-Input` HTTP field +
`chunk-signature` HTTP/1.1 chunk extension, ABNF:

```
chunk-signature  = chunk-name "=" signature
chunk-name       = "chunk-signature"
signature        = DQUOTE signature-key "=:" signature-value ":" DQUOTE
signature-key    = string
string           = 1*(ALPHA / DIGIT / "-")
signature-value  = base64-str
base64-str       = 1*(ALPHA / DIGIT / "+" / "/") *("=")
```
A "chunked component" beginning with `#` names a chunk extension that must be present
in every chunk and is covered by the chunk signature; `#@chunk-size` may cover the
chunk-size indicator itself. Each chunk signature covers at minimum the
`Chunked-Signature-Input` value and a chunk digest (clause 12.1.2); should additionally
cover the transport object URI.

**Enforcement rule** (clause 12.2.3, normative): if a session's transport security mode
is `integrityAndAuthenticity` (clause 10.2.3.6) and a received transport object lacks a
usable signature, the gateway **shall** treat the whole object as lost and retrieve it
via reference point `A` if available.

## 5. Unicast repair (clause 9, protocol-agnostic HTTP layer)

Triggered (clause 9.1) on: detected loss with no FEC / FEC not used by the gateway; no
packets received within `@transportObjectReceptionTimeout` (clause 10.2.3.12); a
protocol-specific timeout expiry; a presentation-timeline deadline; or multicast-session
end. After triggering, the gateway waits `@fixedBackOffPeriod` then a random delay in
`[0, @randomBackOffPeriod]` before repairing (clause 10.2.3.12) — at the latest,
`min(transportObjectReceptionTimeout, protocol timeout) + fixedBackOffPeriod +
randomBackOffPeriod`.

**Protocol** (clause 9.2): HTTP/1.1 mandatory at `A`; HTTP/2 (RFC 9113)/HTTP/3 (RFC 9114)
optional; HTTPS mandatory for HTTP/3. Byte-range requests (RFC 9110 §14) mandatory.

- **Base URL selection** (clause 9.2.1): when multiple `UnicastRepairParameters/BaseURL`
  values are configured, the gateway picks by relative weight (an example weighted
  random-selection algorithm is given, not mandated).
- **URI mapping** (clause 9.2.2): strip the `transportObjectBaseURI` prefix from the
  transport object URI, prepend the selected unicast repair `BaseURL`.
- **No-metadata case** (clause 9.2.3): construct the unicast URL directly from the
  presentation manifest when no object metadata (from `M` or `A`) exists yet.
- **Message format** (clause 9.2.4): full object -> plain `GET`; partial -> `GET` with a
  `Range` header (may combine multiple byte ranges in one request); an `If-Range`
  conditional request uses a previously-seen strong `ETag` (RFC 9110 §13.1.5/§8.8.3.2)
  when available. Response per RFC 9110 §15.3.6; may carry `Content-Digest`/`Repr-Digest`
  and/or `Signature-Input`/`Signature` fields, validated before use if present.

## 6. Overlap summary (for the implementer / delegate brief)

- **Do not re-implement**: LCT header (RFC 5651), ALC packet + `EXT_FTI` (RFC 5775),
  FLUTE `EXT_FDT`/`EXT_CENC` (RFC 6726) — use `rmt-flute` (published crate in this
  workspace).
- **Blocked / needs coordination**: ROUTE packet profiling (`EXT_TOL`, `EXT_TIME`,
  Codepoint dispatch, S-TSID/MPD parsing) is ATSC A/331 territory — issue **#750**. If
  `dvb-mabr`'s Annex H support ships before #750, it will need its own minimal ROUTE
  primitives and should avoid names/types that would collide with #750's eventual crate.
- **New work specific to this crate**: the Extended FDT XML (§2.3-2.5), the
  chunked-transmission-mode URI convention (§2.2), the HTTP digest/message-signature
  profiles (§4) as applied to multicast metadata, and the unicast repair HTTP client
  logic (§5) — none of these exist elsewhere in the workspace.
