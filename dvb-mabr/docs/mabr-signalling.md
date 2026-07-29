# DVB-MABR multicast session configuration — ETSI TS 103 769 V1.2.1 clause 10, Annexes A, B

Source: ETSI TS 103 769 V1.2.1 (2024-11). See [`README.md`](README.md) for provenance.

This is the field/element/attribute reference for the **multicast session configuration
instance document** — the one XML document format the whole system is coordinated by
(clause 10.0). Two flavours share the same schema and clause numbering:

- **Multicast server configuration** — root element `MulticastServerConfiguration`, sent
  to the Multicast server at reference point `CMS`.
- **Multicast gateway configuration** — root element `MulticastGatewayConfiguration`,
  sent to the Multicast gateway at reference point `CMR` (or piggybacked at `B`/`M`, see
  §1).

Both are `application/xml+dvb-mabr-session-configuration` (clause 10.2.0). Presentation
manifests themselves (DASH MPD / HLS Master Playlist) are **referenced by URL**, not
embedded — their own formats (ISO/IEC 23009-1, IETF RFC 8216) are out of scope of TS 103
769 and of this transcription; that is `transmux`'s territory in this workspace.

## 1. How a configuration reaches its consumer (clause 10.1, normative)

**Multicast server** — exactly one method:
1. Out-of-band **pushed** (clause 10.4.2.1): Network control delivers the document over `CMS`.
2. Out-of-band **pulled** (clause 10.4.2.2): the server polls `CMS` periodically.

**Multicast gateway** — one or more of:
1. Out-of-band **pushed** (clause 10.4.4.1) over `CMR`.
2. Out-of-band **pulled** (clause 10.4.4.2) over `CMR`.
3. **In-band** (clause 10.4.5): the Multicast server carousels the current gateway
   configuration document as a multicast transport object on a dedicated "multicast
   gateway configuration transport session" at `M` (clause 8.3.5) — mandatory for
   unidirectional deployments. A one-off "bootstrap" gateway configuration document
   (containing just the `MulticastGatewayConfigurationTransportSession` element(s)
   needed to find that session) may be delivered out-of-band first (example: Annex C.2).
4. **Just-in-time** (clause 7.5.2.1): the Multicast rendezvous service piggybacks a
   single-`MulticastSession` gateway configuration in its `conf` redirect query
   parameter (Gzip + base64url, see [`mabr-architecture.md`](mabr-architecture.md) §5.3).

## 2. Document root (clause 10.2.1, Table 10.2.1.1-1 / Table 10.2.1.2-1)

Both roots declare `@schemaVersion` (unsigned integer, required — the highest value in
Annex A Table A.0-1 the document conforms to; **current value: `2`**, baseline namespace
`urn:dvb:metadata:MulticastSessionConfiguration:2024`) and a validity, either
`@validityPeriod` (ISO 8601-1 §5.5.2 duration, excluding the §5.5.2.4 alternative form)
or `@validUntil` (MPEG-7 Part 5 §6.4.3 `TimePoint` — a restricted ISO 8601-1 §5.4 subset
that **cannot express a bare "Z" UTC designator**; an absent time zone means `+00:00`,
not local time). If both are present, the later expiry wins. A gateway configuration
document delivered via the in-band carousel (§1 method 3) must **not** carry
`@validityPeriod`.

| Element/attribute | Use | Data type | Root | Description |
|---|---|---|---|---|
| `@schemaVersion` | 1 | Unsigned integer | both | Schema version (Annex A.0). |
| `@validityPeriod` | 0..1 | Duration | both | Relative expiry. |
| `@validUntil` | 0..1 | dateTime (MPEG-7 TimePoint) | both | Absolute expiry. |
| `MulticastGatewayConfigurationTransportSession` | 0..n | — | both | See §7. |
| `MulticastSession` | 0..n | — | both | See §3. |
| `MulticastServerConfigurationMacro` | 0..n | String | server only | Macro-expansion value; `@key` (1, NameToken) is the key. |
| `MulticastGatewaySessionReporting` | 0..1 | — | both | Document-wide reporting destinations; see §6. |

## 3. `MulticastSession` element (clause 10.2.2, Table 10.2.2.1-1)

One `MulticastSession` = all multicast transport sessions delivering one linear
service's components.

| Element/attribute | Use | Data type | Description |
|---|---|---|---|
| `@serviceIdentifier` | 1 | URI string | Unique service ID within the deployment. |
| `@contentPlaybackAvailabilityOffset` | 0..1 | Duration (default `PT0S`) | Delay applied to the presentation timeline exposed to playback, to allow for repair time. |
| `PresentationManifestLocator` | 1..n | URI string (element content = the manifest URL) | See §3.1. |
| `MulticastGatewaySessionReporting` | 0..1 | — | Per-session reporting destinations; see §6. |
| `MulticastTransportSession` | 0..n | — | See §4. |

### 3.1 `PresentationManifestLocator` (clause 10.2.2.2, Table normative prose)

| Attribute | Use | Data type | Description |
|---|---|---|---|
| `@manifestId` | 1 | NameToken | Unique within the parent `MulticastSession`; cross-referenced by `ServiceComponentIdentifier/@manifestIdRef` (§5). |
| `@contentType` | 1 | MPEG-7 mimeType | `application/dash+xml` (DASH MPD) or `application/vnd.apple.mpegURL`/`audio/mpegurl` (HLS Master Playlist). |
| `@transportObjectURI` | 0..1 | URI string, unique in the document | Transport object URI to use when this manifest is carouselled in-band. |
| `@contentPlaybackPathPattern` | 0..1 | String | Wildcard pattern (`*` = any pchar run, `?` = one pchar; literal `$`/`*`/`?` escaped with a leading `$`) matched against the request path (including leading `/`) at `L`, letting the gateway associate an inbound manifest request with this session. |

Element content semantics differ by document type: in a **server** configuration it is
the `Pin'`/`Oin` URL to push to / pull from; in a **gateway** configuration it is the
`A` URL for unicast retrieval/repair, or **empty** (with `@contentPlaybackPathPattern`
then mandatory non-empty) if reference point `A` is not present in the deployment.

## 4. `MulticastTransportSession` element (clause 10.2.3, Table 10.2.3.1-1 — the core element)

| Element/attribute | Use | Data type | Description |
|---|---|---|---|
| `@id` | 1 | NameToken | Unique within the parent `MulticastSession`. |
| `@serviceClass` | 0..1 | MPEG-7 termReference | Content-class term, e.g. from TS 103 770 §9 vocabulary (DVB-I). Unknown terms: gateway should ignore the session. |
| `@start` | 0..1 | MPEG-7 TimePoint | Session start (clause 10.3.1). |
| `@duration` | 0..1 | Duration | Session duration. |
| `@contentIngestMethod` | 0..1 | `push` \| `pull` (default `pull`) | Server-config only; a gateway must ignore it if present. |
| `@transmissionMode` | 0..1 | `resource` \| `chunked` (default `resource`) | See `mabr-transport.md` §1. |
| `@transportSecurity` | 0..1 | `none` \| `integrity` \| `integrityAndAuthenticity` (default `none`) | See `mabr-transport.md` §4. |
| `@sessionIdleTimeout` | 1 | Unsigned integer, ms | Max inter-packet gap before the gateway may treat the session as inactive/unsubscribe. Takes precedence over other timeouts. |
| `TransportProtocol` | 1 | — | `@protocolIdentifier` (1, MPEG-7 termReference, a `MulticastTransportProtocolCS` term — §8) + `@protocolVersion` (1, `xs:positiveInteger`, major version number). |
| `EndpointAddress` | 1..n | — | See §4.1. |
| `BitRate` | 1 | — | `@average` (0..1, positive integer, bit/s) + `@maximum` (1, positive integer, bit/s) — across all endpoints declared for this session, including any FEC repair packets addressed to the **same** destination group network address (clause 10.2.3.10). If FEC uses a different endpoint address, its bit rate is not included here. |
| `ForwardErrorCorrectionParameters` | 0..n | — | See §4.2. |
| `UnicastRepairParameters` | 0..1 | — | See §4.3. |
| `ObjectCarousel` | 0..1 | — | See §4.4. |
| `ServiceComponentIdentifier` | 1..n | — | See §5. |

### 4.1 `EndpointAddress` (clause 10.2.3.9)

| Element | Use | Data type | Description |
|---|---|---|---|
| `NetworkSourceAddress` | 0..1 | String (IPv4 dotted-decimal or IPv6 per RFC 5952) | Source address, source-specific multicast. |
| `NetworkDestinationGroupAddress` | 1 | IP address string | Multicast group. |
| `TransportDestinationPort` | 1 | Unsigned 16-bit (1-65535) | UDP destination port. |
| `MediaTransportSessionIdentifier` | 0..1 | Positive integer | Protocol-specific demux id (e.g. the LCT Transport Session Identifier / Channel). |

### 4.2 `ForwardErrorCorrectionParameters` (clause 10.2.3.11)

| Element | Use | Data type | Description |
|---|---|---|---|
| `SchemeIdentifier` | 1 | MPEG-7 termReference | AL-FEC scheme, a `ForwardErrorCorrectionSchemeCS` term (§8.2). |
| `OverheadPercentage` | 1 | Positive integer | FEC overhead vs. source packets (20 = 20%, 100 = one repair packet per source packet; values >100 permitted). |
| `EndpointAddress` | 0..n | — | Only if repair packets use a *different* endpoint than the source session (§4.1). |

Semantics of an **omitted** `ForwardErrorCorrectionParameters` element are
protocol-specific: FLUTE => Compact No-Code FEC in use; ROUTE => no Repair Flow protects
the session (`mabr-transport.md` §2.1/§3.1).

### 4.3 `UnicastRepairParameters` (clauses 10.2.3.12-10.2.3.13)

| Attribute/element | Use | Data type | Description |
|---|---|---|---|
| `@transportObjectBaseURI` | 0..1 | Absolute URI, no query/fragment | Prefix stripped from the transport object URI before repair-URL construction. |
| `@transportObjectReceptionTimeout` | 1 | Unsigned integer, ms | Wait time before assuming object transmission is over. |
| `@fixedBackOffPeriod` | 0..1 | Unsigned integer, ms (default 0) | Fixed delay before repair. |
| `@randomBackOffPeriod` | 0..1 | Unsigned integer, ms (default 0) | Upper bound of an additional random delay (chosen per-object). |
| `BaseURL` | 0..n | Absolute URI, no query/fragment | Unicast repair endpoint prefix. |
| `BaseURL/@relativeWeight` | 0..1 | Non-negative integer (default 1) | Selection weight; `0` = disabled. Omitted on all `BaseURL`s => equal weight. |

If no `BaseURL` is present, the repair URL is built directly per `mabr-transport.md` §5.

### 4.4 `ObjectCarousel` (clause 10.2.3.14)

| Element/attribute | Use | Data type | Description |
|---|---|---|---|
| `@aggregateTransportSize` | 0..1 | Unsigned integer | Combined size as transmitted at `M` (excl. metadata/protocol overhead). |
| `@aggregateContentSize` | 0..1 | Unsigned integer | Combined size after removing content-encoding (e.g. compression); omit if none applied. |
| `PresentationManifests` | 0..1 | — | Presence => carousel all manifests for this session's service components. `@targetAcquisitionLatency` (0..1, Duration), `@compressionPreferred` (0..1, Boolean, server-config only). |
| `InitSegments` | 0..1 | — | Presence => carousel current-Period init segments (DASH) / `EXT-X-MAP` sections (HLS). Same two attributes as above. |
| `ResourceLocator` | 0..n | URI string | An arbitrary resource URL to carousel. `@targetAcquisitionLatency`, `@revalidationPeriod` (0..1, Duration — origin revalidation interval), `@compressionPreferred`. |

`@targetAcquisitionLatency` omitted => repeat as often as the session bit rate allows.

## 5. Service component association (clause 10.2.4)

Every `MulticastTransportSession` carries 1..n `ServiceComponentIdentifier` elements,
each referencing a `PresentationManifestLocator/@manifestId` via `@manifestIdRef` and
typed by `@xsi:type`:

**Table 10.2.4.1-1: `DASHComponentIdentifierType`**

| Attribute | Use | Data type | Description |
|---|---|---|---|
| `@manifestIdRef` | 1 | NameToken | -> `PresentationManifestLocator/@manifestId` of an MPD. |
| `@periodIdentifier` | 1 | String | -> `Period/@id`. |
| `@adaptationSetIdentifier` | 1 | Unsigned integer | -> `AdaptationSet/@id`. |
| `@representationIdentifier` | 1 | String (no whitespace) | -> `Representation/@id`. |

**Table 10.2.4.2-1: `HLSComponentIdentifierType`**

| Attribute | Use | Data type | Description |
|---|---|---|---|
| `@manifestIdRef` | 1 | NameToken | -> `PresentationManifestLocator/@manifestId` of an HLS Master Playlist. |
| `@mediaPlaylistLocator` | 1 | URI string | Absolute URL of the referenced HLS Media Playlist. |

A third type, `GenericComponentIdentifierType` (`@manifestIdRef` + `@componentIdentifier`,
a NameToken), covers manifest types outside DASH/HLS.

## 6. Reporting parameters (clauses 10.2.1.0, 10.2.2.3 — element `MulticastGatewaySessionReporting`)

Declared at document root (all sessions) and/or per-`MulticastSession` (that session
only); both may be active simultaneously.

| Element/attribute | Use | Data type | Description |
|---|---|---|---|
| `ReportingLocator` | 1..n | URI string (element content = endpoint URL) | A reporting destination. |
| `@proportion` | 0..1 | Decimal (0.0, 1.0] (default 1.0) | Sampled fraction of gateways that report to this endpoint. |
| `@period` | 1 | Duration | Gap between reports; `0` disables periodic reporting (event-only). |
| `@randomDelay` | 1 | Unsigned integer, ms | Extra random delay added after `@period`. |
| `@reportSessionRunningEvents` | 0..1 | Boolean (default false) | Whether "running" events (heartbeats etc.) are included. |

The report body itself is a **JSON** document (clause 11.1, `Content-Type:
application/json`) POSTed to `http[s]://<Host>/dvb/mabr/reportingInformationInstance`
(clause 11.2.1). This is now **fully transcribed** in
[`mabr-reporting.md`](mabr-reporting.md), drawn from the clean, table-aware
conversion of normative **Annex N** (the complete OpenAPI 3.0.1 YAML schema,
pages 179-183) — which the initial transcription pass missed because Annex N
was not detected. See that document for the complete schema, the seven event
types, and a flagged spec-internal conflict between clause 11.1.2.2's
`object-delivery-status` enumeration (10 values) and Annex N's (9 values).

## 7. `MulticastGatewayConfigurationTransportSession` (clause 10.2.5, Table 10.2.5.1-1)

Used only for the in-band configuration method (§1 method 3). Same element/attribute set
as `MulticastTransportSession` (§4) **except**: no `@id`/`@start`/`@duration`/
`@contentIngestMethod`/`@transmissionMode`, no `ServiceComponentIdentifier`; instead adds:

| Element/attribute | Use | Data type | Description |
|---|---|---|---|
| `@tags` | 0..1 | Whitespace-separated URI list | Applicability tags a gateway can filter on. |
| `ObjectCarousel` | 0..1 | (`ReferencingObjectCarouselType`) | As §4.4 but with a different child element type: `PresentationManifests` and `InitSegments` each have cardinality **`0..n`** (vs `0..1` in the base `MulticastTransportSession`), allowing the carousel to serve manifests/init-segments from multiple sessions. Each child additionally takes `@serviceIdRef` (0..1, URI — target `MulticastSession/@serviceIdentifier`; omitted = all active sessions) and `@transportSessionIdRef` (0..1 — target `MulticastTransportSession/@id` within that session; illegal without `@serviceIdRef`). |
| `MulticastGatewayConfigurationMacro` | 0..n | String, `@key` (1, NameToken) | Per-transport-session macro override (see §9). |

## 8. Classification schemes (Annex B, normative — controlled vocabularies)

### 8.1 `MulticastTransportProtocolCS` (clause B.1)

| `termID` | Namespace year | Meaning |
|---|---|---|
| `FLUTE` | `:2019:` | IETF RFC 3926 v1, as profiled by 3GPP TS 26.346 R16 + this spec's Annex F. |
| `ROUTE` | `:2019:` | ATSC A/331, as profiled by this spec's Annex H. |
| `NORM` | `:2022:` | IETF RFC 5740, as profiled by this spec (no annex — **not further specified in the normative body**; recorded in the CS only). |
| `MSync` | `:2022:` | `draft-bichot-msync-06` (Internet-Draft). Contains a **child term** `RTP`: MSync conveyed over RTP. |
| | `:2022:` — child: `MSync/RTP` | Nested under `MSync`; not a flat sibling term. |

Full term URI form: `urn:dvb:metadata:cs:MulticastTransportProtocolCS:<year>:<termID>`. The `MSync/RTP` term is addressed as `MSync.RTP` in classification-scheme term-path notation (since `RTP` is a child `<Term>` of `MSync` in the XML schema `MulticastTransportProtocolCS_2022.xml`, Annex B.1).

### 8.2 `ForwardErrorCorrectionSchemeCS` (clause B.2)

A subset of the IANA "FEC Encoding IDs for Reliable Multicast Transport" registry, URI
namespace `urn:ietf:rmt:fec:encoding`:

| `termID` | Scheme | Reference |
|---|---|---|
| `0` | Compact No-Code FEC Scheme | IETF RFC 5445 §3 |
| `1` | Raptor FEC Scheme for Object Delivery | IETF RFC 5053 |
| `2` | Reed-Solomon FEC Scheme, GF(2^m) | IETF RFC 5510 |
| `5` | Reed-Solomon FEC Scheme, GF(2^8) | IETF RFC 5510 |
| `6` | RaptorQ FEC Scheme for Object Delivery | IETF RFC 6330 |

### 8.3 `MulticastTransportObjectTypeCS` (clause B.3)

| `termID` | Meaning |
|---|---|
| `gateway-configuration` | Fixed transport object URI value `urn:dvb:metadata:cs:MulticastTransportObjectTypeCS:2021:gateway-configuration` used for a multicast gateway configuration document carried in-band (clause 8.3.5). |

## 9. Extensibility mechanism (Annex A.1, normative)

Two independent extension axes, both requiring a namespace different from the baseline
(`urn:dvb:metadata:MulticastSessionConfiguration:2024`):

- **Extension elements**: an `xs:any namespace="##other" processContents="skip"
  minOccurs="0" maxOccurs="unbounded"` at each of a fixed, enumerated set of extension
  points (**Table A.1.5-1**: `EndpointAddress`, `ForwardErrorCorrectionParameters`,
  `InitSegments`, `MulticastGatewayConfiguration`,
  `MulticastGatewayConfigurationTransportSession`, `MulticastGatewaySessionReporting`,
  `MulticastServerConfiguration`, `MulticastSession`, `MulticastTransportSession`,
  `ObjectCarousel`, `PresentationManifests`, `ReportingLocator`, `ResourceLocator`,
  `ServiceComponentIdentifier`, `TransportProtocol`, `UnicastRepairParameters`). This set
  is closed: a future spec revision may add a *new* type to the table but must never
  retroactively add an extension point to a type not already listed (breaks
  older-parser forward-compatibility).
- **Extension attributes**: an `xs:anyAttribute processContents="skip"` wherever present
  in the baseline schema.
- **Standardized extensions** (i.e. new elements a *future TS 103 769 revision* adds,
  as opposed to a private/implementation extension): live in their own namespace per
  schema-version bump, terminated by a `NamespaceDelimiter` marker element
  (`urn:dvb:metadata:Extensibility:2024`) so older parsers can still skip forward past
  them without ambiguity (XML Schema 1.0 Unique Particle Attribution). `@schemaVersion`
  increments by 1 every time a revision adds standardized extension element(s); Annex A
  Table A.0-1 lists the namespace set required per version:

  | Schema version | Required schema namespaces |
  |---|---|
  | `1` | `urn:dvb:metadata:MulticastSessionConfiguration:2019` |
  | `2` (current) | `urn:dvb:metadata:MulticastSessionConfiguration:2024` |

  Note: version 1 requires **only** the 2019 baseline namespace (no Extensibility
  namespace is listed for v1 in Table A.0-1). The Extensibility schema
  (`urn:dvb:metadata:Extensibility:2024`) is used at the instance-document level
  for the `@schemaVersion` attribute declaration under *both* versions (via
  `xs:import`) but is not separately listed as a required *schema* namespace in
  Table A.0-1's v1 row.
- A private/implementation extension (any other namespace) is always permitted after
  the last standardized extension (or immediately, if none), and is silently skippable
  by any conformant parser regardless of schema version.

The full baseline `A.2` XSD (all complex/simple types: `IPAddressType`,
`PortNumberType`, `contentAcquisitionMethodType` (`push`/`pull`),
`transmissionModeType` (`resource`/`chunked`), `transportSecurityType`
(`none`/`integrity`/`integrityAndAuthenticity`), `BitRateType`,
`ForwardErrorCorrectionParametersType`, `UnicastRepairParametersType`,
`ObjectCarouselType`, `MulticastTransportSessionType`, `MulticastSessionType`,
`MulticastServerConfigurationType`, `MulticastGatewayConfigurationType`, etc.) was
verified read cleanly from the text layer (pdf2md exit code 0 — no digit/hex-token
mismatches flagged) and is reproduced verbatim in the vendored PDF, Annex A.2; the field
tables above (§2-§7) are the same information in the project's house `Use`/`Data
type`/`Description` table style and should be treated as authoritative for
implementation — cross-check against the XSD text directly if a discrepancy is
suspected.

## 10. Worked example (Annex C.1, informative — not reproduced verbatim)

Annex C.1 (spec pages ~119-125) gives a complete, real multi-service example: two
`MulticastGatewayConfigurationTransportSession`s (one FLUTE, one ROUTE, each dual-stack
IPv4+IPv6, Raptor/RaptorQ FEC respectively) plus two `MulticastSession`s ("BBC One
Scotland", "BBC Two Scotland") each with DASH+HLS manifests, multiple
`MulticastTransportSession`s (video/audio, FLUTE push+chunked or ROUTE pull+resource),
per-session unicast repair `BaseURL` weighting, in-band object carousels, and macro
expansion (`$PrimaryCDN$`/`$SecondaryCDN$` substituted via
`MulticastServerConfigurationMacro`/`MulticastGatewayConfigurationMacro`, clause
10.2.5.2). Recommended as the primary fixture source for round-trip tests: it exercises
nearly every element in §2-§8 in one document. Extract it directly from the vendored PDF
(Annex C.1) rather than retyping — this transcription only summarizes its shape.
