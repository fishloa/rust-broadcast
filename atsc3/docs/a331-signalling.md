# A/331 — Low Level Signaling (LLS) and Service Layer Signaling (SLS)

Source: ATSC A/331:2025-06, "Signaling, Delivery, Synchronization, and Error Protection", 18
June 2025 — §6 (LLS, pp. 25-49), §7.1 (ROUTE/DASH SLS, pp. 57-78), Annex H (media type
registrations, pp. 232-238). See [`README.md`](README.md) for provenance. The MMTP-specific SLS
(§7.2) is covered separately in [`a331-mmt.md`](a331-mmt.md).

All fragments below are **XML documents**, not binary bit-syntax — each is described as an
element/attribute table (`Use` = XML-schema-style cardinality: `1` required-once, `0..1`
optional-once, `0..N`/`1..N` optional/required-repeating), matching how A/331 itself presents
them ("informative Table N.M describes the structure... in a more illustrative way" — the
*normative* syntax is the accompanying `.xsd` schema file, which this repo does not have
vendored; see `README.md`). Only the elements/attributes actually exercised by a first ROUTE
implementation are transcribed in full; deprecated/backwards-compat-only fields are noted but
not exhaustively detailed.

## 1. How the pieces reference each other (top-down)

1. **LLS** (§2 below) is transmitted at a well-known multicast address/port, independent of any
   particular Service. It carries the **SLT** (§3), among other LLS tables.
2. The **SLT** is the bootstrap into everything else: for each Service it gives the
   destination IP/port of the LCT channel (ROUTE) or MMTP session carrying that Service's
   **SLS** (§4) — the entry point for per-Service technical descriptions.
3. For a ROUTE/DASH Service, the SLS is a bundle of XML fragments carried together
   (`multipart/related`) on a dedicated LCT channel with **TSI=0**: **USBD** (top-level,
   references the rest), **S-TSID** (transport-session/LCT-channel descriptions, and via
   `SrcFlow`/`RepairFlow` the ROUTE-specific structures documented in
   [`a331-route.md`](a331-route.md)), **MPD** (DASH manifest, external DASH-IF spec, not
   transcribed here), **APD** (HTTP file-repair parameters), **HELD** (app entry-page
   metadata), **DWD** (NRT file distribution-window schedule).

## 2. Low Level Signaling (LLS) — §6

### 2.1 Transport (§6.1, normative)

- LLS is transported in UDP/IP packets to **multicast address `224.0.23.60`, destination port
  `4937/udp`** (IANA-assigned `AtscSvcSig`/`atsc-mh-ssc`).
- The first byte of every such UDP/IP packet is the start of an `LLS_table()`.
- Max LLS table size = 65,507 bytes (max UDP payload given IP+UDP header overhead).
- Non-LLS IP destination addresses: either (a) uniquely reserved in the geographic Service
  Area, or (b) in `239.255.0.0`-`239.255.255.255` (RFC 2365 IPv4 Local Scope) with the third
  octet equal to a `SLT.Service@majorChannelNo` value registered to the broadcaster. All
  non-LLS destination ports must be `> 1024`.

### 2.2 `LLS_table()` common envelope

#### Table 6.1 — Common Bit Stream Syntax for LLS Tables
_§6.2, A/331:2025-06 p.27-28_

| Syntax | No. of Bits | Format |
|---|---|---|
| `LLS_table() {` |
| `LLS_table_id` | 8 | uimsbf |
| `LLS_group_id` | 8 | uimsbf |
| `group_count_minus1` | 8 | uimsbf |
| `LLS_table_version` | 8 | uimsbf |
| `switch (LLS_table_id) {` |
| `case 0x01: SLT` | var | (§3 below) |
| `case 0x02: RRT` | var | Annex F (not transcribed — Rating Region Table, ATSC A/331 Annex F) |
| `case 0x03: SystemTime` | var | §6.4 (not transcribed in full — see "could not establish") |
| `case 0x04: AEAT` | var | §6.5, Advanced Emergency Alerting Table (not transcribed — large, security/alerting-specific, out of first-cut ROUTE scope) |
| `case 0x05: OnscreenMessageNotification` | var | §6.6 (not transcribed) |
| `case 0xFE: SignedMultiTable` | var | §6.7 (not transcribed) |
| `case 0xFF: UserDefined` | var | §6.8 (not transcribed) |
| `default: reserved` | var | |
| `}` |
| `}` |

- **`LLS_table_id`** — `0x00` ATSC Reserved; undefined values Industry Reserved (see the ATSC
  Code Point Registry — external, not vendored). Additional LLS table IDs exist outside this
  list (e.g. A/360's CertificationData, A/323's Dedicated Return Channel Table).
- **`LLS_group_id`** — associates this `LLS_table()` instance with a group of tables sharing the
  same ID; scope is the broadcast stream; must be coordinated to avoid collision.
- **`group_count_minus1`** — one less than the total number of distinct `LLS_group_id` values
  present in this PLP's ALP packet stream.
- **`LLS_table_version`** — increments by 1 (mod 256, wrapping `0xFF -> 0x00`) whenever the
  identified table's data changes.
- Each table body (SLT/RRT/SystemTime/AEAT/OnscreenMessageNotification/UserDefined) is
  individually gzip-compressed (RFC 1952) XML; `SignedMultiTable` itself is not gzip-compressed
  (its constituent tables are, per their own rules).

### 2.3 Signing policy (§5.9, normative — precedes §6 in the source but governs it)

- All LLS tables defined by A/331 **shall** be transmitted signed via `SignedMultiTable`
  (A/360 §5.2.2.3); they *may additionally* be transmitted unsigned outside
  `SignedMultiTable` under their own `LLS_table_id`. Repetition-rate requirements (e.g. SLT
  every <=5s) apply separately to each form present.
- ROUTE SLS fragments shall be signed per A/360 §5.2.2.4 (MMT SLS per §5.2.2.5) **unless**
  delivered over secure bidirectional broadband (HTTPS, A/360 §5.1.1).
- A/360's `CertificationData` LLS table shall be present whenever signed data is in use.

## 3. Service List Table (SLT) — §6.3

Root element `SLT`, namespace `tag:atsc.org,2016:XMLSchemas/ATSC3/Delivery/SLT/1.0/`, schema
file `SLT-1.0-20211209.xsd` (not vendored here). Function is analogous to MPEG-2 PAT / ATSC
A/153 FIC: the rapid-channel-scan bootstrap. **Must repeat at least once every 5 seconds**
(recommended: every 1 second, need not exceed that rate).

### Table 6.2 — SLT XML Format
_§6.3, A/331:2025-06 p.30-31_

| Element / Attribute | Use | Data Type | Description |
|---|---|---|---|
| `SLT` | | | Root element. |
| `@bsid` | 1 | list of unsignedShort | Broadcast Stream ID(s); matches `L1D_bsid` in physical-layer L1-Detail signaling (A/322). Multiple values when the Service is channel-bonded. |
| `SLTCapabilities` | 0..1 | `sa:CapabilitiesType` | Required decode/presentation capabilities for all Services in this SLT (same syntax as A/332's `sa:Capabilities`). |
| `SLTInetUrl` | 0..N | anyURI | Base URL to acquire ESG/SLS via broadband, for all Services. |
| `SLTInetUrl@urlType` | 1 | unsignedByte | See Table 6.3 below. |
| `Service` | 1..N | | One Service's info. |
| `Service@serviceId` | 1 | unsignedShort | Uniquely identifies the Service within one set of bonded PLPs. |
| `Service@globalServiceID` | 0..1 | anyURI | Globally-unique Service URI; matches the A/332 ESG `Service@globalServiceID`. Required for Linear A/V, Linear-audio, app-based Services; required + EIDR/`tag:`-formed when `@serviceCategory="Data"`. |
| `Service@sltSvcSeqNum` | 1 | unsignedByte | Per-`serviceId` version counter; increments on any change to this `Service` element/children; wraps at max. |
| `Service@protected` | 0..1 | boolean | Whether >=1 Component needed for meaningful presentation is protected. Default `false`. |
| `Service@majorChannelNo` | 0..1 | unsignedShort (1-999) | Major channel number (A/65 Annex B allocation rules). Not required for non-user-selectable Services (e.g. ESG). |
| `Service@minorChannelNo` | 0..1 | unsignedShort (1-999) | Minor channel number. |
| `Service@serviceCategory` | 1 | unsignedByte | Service type — Table 6.4 below. |
| `Service@shortServiceName` | 0..1 | string (<=7 chars) | Short display name. |
| `Service@hidden` | 0..1 | boolean | Not directly channel-surfable/enterable (test signals, NVOD). Default `false`. |
| `Service@hideInGuide` | 0..1 | boolean | Whether shown in ESG displays (ignored unless `@hidden="true"`). Default `false`. |
| `Service@broadbandAccessRequired` | 0..1 | boolean | Broadband needed for meaningful presentation. Default `false`. |
| `Service@essential` | 0..1 | boolean | Present => this Service spans >1 RF channel; `true` = this stream carries the essential portion (requires >=1 `OtherBsid@type="2"`); `false` = non-essential portion; absent = all portions are in this stream. |
| `Service@drmSystemID` | 0..1 | list of anyURI | DRM System ID(s) (`urn:uuid:...`, matching the MPD `ContentProtection` `@schemeIdUri`). Required (single value) when `@serviceCategory=6` (DRM Data Service). |
| `Service@configuration` | 0..1 | token (`"Broadband"` \| `"Broadcast"`) | Service Configuration; defaults per §8.2.1.2 rules when absent. |
| `Service.CodecStrings@codecs` | 1 (under `CodecStrings`, 0..N) | — | Delimited MPD `@codec` strings (RFC 6381). |
| `Service.SimulcastTSID` | 0..1 | unsignedShort | ATSC 1.0 TSID of a simulcast emission (A/65 §6.3 `channel_TSID`). |
| `Service@simulcastMajorChannelNo` / `@simulcastMinorChannelNo` | 0..1 each | unsignedShort (1-999) | ATSC 1.0 simulcast channel numbers. |
| `Service.SvcCapabilities` | 0..1 | `sa:CapabilitiesType` | Per-Service capabilities (same shape as `SLTCapabilities`). |
| `Service.BroadcastSvcSignaling` | 0..1 | | Broadcast SLS bootstrap info — see below. |
| `BroadcastSvcSignaling@slsProtocol` | 1 | unsignedByte | Table 6.5 below (1=ROUTE, 2=MMTP). |
| `BroadcastSvcSignaling@slsMajorProtocolVersion` | 0..1 | unsignedByte | Default `1`. |
| `BroadcastSvcSignaling@slsMinorProtocolVersion` | 0..1 | unsignedByte | Default `0`. |
| `BroadcastSvcSignaling@slsDestinationIpAddress` | 1 | IPv4 address (dotted, RFC 3986 §3.2.2) | Destination IP of the SLS-carrying LCT channel/MMTP session. |
| `BroadcastSvcSignaling@slsDestinationUdpPort` | 1 | unsignedShort | Destination port of the same. |
| `BroadcastSvcSignaling@slsSourceIpAddress` | 0..1 | IPv4 address | Source IP; **required when `@slsProtocol=1`** (ROUTE). |
| `Service.SvcInetUrl` | 0..N | anyURI | Per-Service broadband ESG/SLS URL. |
| `SvcInetUrl@urlType` | 1 | unsignedByte | Table 6.3. |
| `Service.OtherBsid` | 0..N | list of unsignedShort | Other Broadcast Stream(s) carrying a duplicate/portion of this Service. |
| `OtherBsid@type` | 1 | unsignedByte | Table 6.6 below. |
| `Service.OtherRf` | 0..N | | Transmission location/strength of the streams named in `OtherBsid`. |
| `OtherRf@OtherBsidRf` | 1 | unsignedShort | Center RF frequency (MHz). |
| `OtherRf@otherBsid` | 1 | unsignedShort | Links to an `OtherBsid` value. |
| `OtherRf@lat` / `@long` | 1 each | float | Transmitter latitude (-90..90) / longitude (-180..<180), WGS-84. |
| `OtherRf@elev` | 1 | integer | Antenna radiation-center height AMSL (m). |
| `OtherRf@erp` | 1 | integer | Effective Radiated Power (kW). |
| `OtherRf.Directional` | 0..N | | Per-heading relative field strength. |
| `Directional@heading` | 1 | nonNegativeInteger (0-359) | Compass heading (degrees). |
| `Directional@strength` | 1 | float | Relative field value at `@heading`. |
| `Directional@haat` | 0..1 | integer | Height Above Average Terrain at `@heading` (m; may be negative). |

#### Table 6.3 — Code Values for `urlType`
_A/331:2025-06 p.32_

| `urlType` | Meaning |
|---|---|
| 0 | ATSC Reserved |
| 1 | URL of Signaling Server (§6.9) |
| 2 | URL of ESG Server (A/332 §5.5.2) |
| 3 | URL of Service Usage Data Gathering Report server (A/333) |
| 4 | URL of Dynamic Event WebSocket Server (A/337) |
| other | ATSC Reserved |

#### Table 6.4 — Code Values for `SLT.Service@serviceCategory`
_A/331:2025-06 p.33_

| `serviceCategory` | Service Type |
|---|---|
| 0 | ATSC Reserved |
| 1 | Linear A/V Service |
| 2 | Linear audio-only Service |
| 3 | App-Based Service |
| 4 | ESG Service (program guide) |
| 5 | (Deprecated) |
| 6 | DRM Data Service |
| 7 | Data Service |
| other | ATSC Reserved |

#### Table 6.5 — Code Values for `BroadcastSvcSignaling@slsProtocol`
_A/331:2025-06 p.34_

| `slsProtocol` | Meaning |
|---|---|
| 0 | ATSC Reserved |
| 1 | ROUTE |
| 2 | MMTP |
| other | ATSC Reserved |

#### Table 6.6 — Code Values for `OtherBsid@type`
_A/331:2025-06 p.36_

| `type` | Meaning |
|---|---|
| 0 | ATSC Reserved |
| 1 | Duplicate |
| 2 | Portion |
| 3 | MFN Cooperating/Appropriate Alternative Service |
| 4-255 | ATSC Reserved |

Note (informative): when `BroadcastSvcSignaling` is absent, either `Service.SvcInetUrl@urlType=1`
or `SLT.SLTInetUrl@urlType=1` must be present instead (broadband-only SLS bootstrap); in the
latter case the URL template must support a `<service_id>` path term.

## 4. Service Layer Signaling (SLS) for ROUTE/DASH — §7.1

When delivered via ROUTE, SLS fragments ride a dedicated LCT channel with **TSI=0**, packaged
`multipart/related` with a leading `metadataEnvelope` (3GPP MBMS §11.1.3) that is **not**
gzip-compressed itself as an envelope rule but the whole package may be gzip-compressed
(§7.1.6.1). SLS-fragment filtering by TOI follows Annex C's rules (not transcribed here — see
"could not establish"); an Extended FDT Instance at TOI=0 must reflect that same TOI encoding.

### 4.1 User Service Bundle Description (USBD) for ROUTE

Root element `BundleDescriptionROUTE`, namespace
`tag:atsc.org,2016:XMLSchemas/ATSC3/Delivery/ROUTEUSD/1.0/`, schema
`ROUTEUSD-1.0-20170920.xsd`. Modeled on 3GPP MBMS's USBD fragment (base attributes/elements from
that spec are not independently verified here — 3GPP TS 26.346 is not vendored). Media type:
`application/route-usd+xml` (Annex H.2, file ext `.rusd`).

#### Table 7.3 — Semantics of the User Service Bundle Description Fragment for ROUTE
_§7.1.3, A/331:2025-06 p.62-63_

| Element / Attribute | Use | Data Type | Description |
|---|---|---|---|
| `BundleDescriptionROUTE` | | | Root element. |
| `UserServiceDescription` | 1 | | One ATSC 3.0 Service. |
| `@serviceId` | 1 | unsignedShort | Matches the referencing `SLT.Service@serviceId`. |
| `@serviceStatus` | 0..1 | boolean | Active/inactive. Default `true`. |
| `Name` | 0..N | string | Service display name. Absent for ESG/DRM Data Services. |
| `Name@lang` | 1 | lang (BCP 47) | Language of `Name`. |
| `ServiceLanguage` | 0..N | lang | Deprecated — backwards-compat only; decoders should ignore. |
| `DeliveryMethod` | 0..N | | Transport info per Component; absent for ESG/DRM Data Services; at least one of the two children below required when present. |
| `DeliveryMethod.BroadcastAppService` | 0..N | | Indicates broadcast (ROUTE) delivery of a Component. |
| `BroadcastAppService.BasePattern` | 1..N | string | Match pattern against Segment request URLs to identify broadcast-delivered content. |
| `DeliveryMethod.UnicastAppService` | 0..N | | Indicates broadband (HTTP) delivery of a Component. |
| `UnicastAppService.BasePattern` | 1..N | string | Match pattern against Segment request URLs to identify broadband-delivered content. |

### 4.2 Service-based Transport Session Instance Description (S-TSID)

Root element `S-TSID`, namespace `tag:atsc.org,2016:XMLSchemas/ATSC3/Delivery/S-TSID/1.0/`,
schema `S-TSID-1.0-20230714.xsd`. Media type `application/route-s-tsid+xml` (Annex H.1, file
ext `.sls`). Describes every ROUTE session + LCT channel carrying this Service's Components,
plus (via `SrcFlow`/`RepairFlow`) the delivery-object and FEC-repair declarations detailed in
[`a331-route.md`](a331-route.md) §5, §7.

#### Table 7.4 — Semantics of the S-TSID Fragment
_§7.1.4, A/331:2025-06 p.65-66_

| Element / Attribute | Use | Data Type | Description |
|---|---|---|---|
| `S-TSID` | | | Root element. |
| `RS` | 1..N | | One ROUTE session. |
| `RS@sIpAddr` | 0..1 | IPv4 address | Source IP; defaults to the SLS-carrying session's source IP; mandatory when this session differs from the SLS session. |
| `RS@dIpAddr` | 0..1 | IPv4 address | Destination IP; same default/mandatory rule. |
| `RS@dPort` | 0..1 | unsignedShort | Destination port; same default/mandatory rule. |
| `RS.LS` | 1..N | | One LCT channel; carries either real-time (Segments/IS) or NRT content, never both. |
| `LS@tsi` | 1 | unsignedInt (32-bit) | TSI, unique within all ROUTE sessions in this S-TSID. |
| `LS@bw` | 0..1 | unsignedInt (32-bit) | Max bitrate (kbit/s, RFC 4566 AS bandwidth modifier semantics) required by this channel across any 1-second window, counting full IP+UDP+ROUTE headers + payload. Absent = unknown. |
| `LS@startTime` | 0..1 | dateTime | Channel start time (RFC 4566 `t=` line semantics, but XSD `dateTime` instead of NTP decimal). Absent = started in the past. |
| `LS@endTime` | 0..1 | dateTime | Channel end time, same semantics. Absent = ends at an undefined future time. |
| `LS.SrcFlow` | 0..1 | `stsid:srcFlowType` | Source flow (see [`a331-route.md`](a331-route.md) §5). Absent = this channel carries only a repair flow. |
| `LS.RepairFlow` | 0..1 | `stsid:rprFlowType` | Repair flow (see [`a331-route.md`](a331-route.md) §7). Absent = this channel carries only a source flow. |

### 4.3 DASH Media Presentation Description (MPD)

Applicable only to Services with DASH-formatted content. §7.1.5 states its structure/semantics
are **identical to the DASH-IF profile's MPD** (external spec, not part of A/331 itself, and
not transcribed here — this workspace's `transmux` crate already has DASH MPD support, see
"Overlap" below). A/331-specific notes on top of plain DASH-IF MPD: Staggercast audio
Representations (a redundant, possibly-lower-quality, time-advanced audio track a receiver can
fail over to — §7.1.5.1) and Content ID assignment for linear content (§7.1.5.2, deferring
entirely to DASH-IF's mechanism).

### 4.4 Associated Procedure Description (APD)

Root element `AssociatedProcedureDescription`, namespace
`tag:atsc.org,2016:XMLSchemas/ATSC3/Delivery/APD/1.0/`, schema `APD-1.0-20170209.xsd`. Media
type `application/route-apd+xml` (Annex H.4, file ext `.rapd`). Governs the optional HTTP
file-repair procedure (a receiver that failed to fully acquire a ROUTE-delivered object can
request the missing bytes from a broadband file-repair server).

#### Table 7.5 — Semantics of the Associated Procedure Description Fragment
_§7.1.7, A/331:2025-06 p.68-69_

| Element / Attribute | Use | Data Type | Description |
|---|---|---|---|
| `AssociatedProcedureDescription` | | | Root element. |
| `PostFileRepair` | 1 | | Temporal parameters for the file-repair procedure. |
| `PostFileRepair@offsetTime` | 0..1 | unsignedLong | Seconds to wait, after end of broadcast transmission of the file, before starting file repair. Absent or `0` = no fixed wait. |
| `PostFileRepair@randomTimePeriod` | 1 | unsignedLong | Seconds-wide window, after `@offsetTime` elapses, within which the receiver picks a uniformly-distributed random additional wait (to spread file-repair-server request load). |

**End-of-transmission / repair-window rules (§7.1.7.2, normative)**: the latest permitted start
of file repair is bounded by the S-TSID's `EFDT.FDT-Instance@Expires`. A receiver may start
earlier upon seeing the LCT header's Close Object flag (`B`, for the file of interest) or Close
Session flag (`A`, before the FDT's nominal `@Expires`) — both flags are already modeled by
`dvb-flute`'s `LctHeader` (see [`a331-route.md`](a331-route.md) §0).

### 4.5 HTML Entry pages Location Description (HELD)

Root element `HELD`, namespace `tag:atsc.org,2016:XMLSchemas/ATSC3/AppSignaling/HELD/1.0/`,
schema `HELD-1.0-20210312.xsd`. Media type `application/atsc-held+xml` (Annex H.5, file ext
`.held`). Signals broadcaster-application entry-page load/unload metadata.

#### Table 7.6 — HTML Entry pages Location Description (HELD) Semantics
_§7.1.8, A/331:2025-06 p.69-72_

| Element / Attribute | Use | Data Type | Description |
|---|---|---|---|
| `HELD` | 1 | | Root; contains one or more `HTMLEntryPackage`. |
| `HTMLEntryPackage` | 1..N | | Properties of one Entry Package. |
| `@appContextId` | 1 | anyURI | Broadcaster Application Context Identifier (resource-sharing scope across apps). |
| `@requiredCapabilities` | 0..1 | `sa:CapabilitiesType` | Extra device capabilities for meaningful rendering, beyond A/344 defaults. |
| `@appRendering` | 0..1 | boolean | For a linear A/V Service (`serviceCategory=1`), whether the broadcaster app may render the Service's Component(s). Default `false`; only meaningful for `serviceCategory=1`. |
| `@clearAppContextCacheDate` | 0..1 | dateTime | Delete all app-context files older than this date/time. |
| `@bcastEntryPackageUrl` | 0..1 | anyURI | Relative URL of a ROUTE-delivered (signed package) Entry Package. Either this or `@bbandEntryPageUrl` (or both) required. |
| `@bcastEntryPageUrl` | 0..1 | anyURI | URL, within the broadcast package, of the Entry Page itself. Required if `@bcastEntryPackageUrl` present. |
| `@bbandEntryPageUrl` | 0..1 | anyURI | Absolute URL of a broadband-delivered Entry Page. Either this or `@bcastEntryPackageUrl` (or both) required; if both present, broadband is attempted first. |
| `@validFrom` | 0..1 | dateTime | Entry Page intended load-start time. Default: now. |
| `@validUntil` | 0..1 | dateTime | Entry Page intended unload time. Default: indefinite. |
| `@coupledServices` | 0..1 | list of unsignedShort | Other `serviceId`s sharing this broadcaster app (caching hint). |
| `@lctTSIRef` | 0..1 | list of unsignedInt | TSI value(s) of the LCT channel(s) carrying this Entry Package's broadcast-delivered files. |
| `@default` | 0..1 | boolean | Marks the default app to launch on Service acquisition; exactly one `HTMLEntryPackage` may set this `true` when multiple are present. Default `false`. |
| `@appId` | 0..1 | anyURI | Disambiguates apps when `@appContextId` alone is insufficient; if used at all, required on every `HTMLEntryPackage`. |

**Distribution timing rule (§7.1.8.2, normative)**: all files referenced by a HELD update must
already be available (broadcast or broadband) at the time of the HELD update itself, even if
`@validFrom` names a launch time far in the future.

### 4.6 Distribution Window Description (DWD)

Root element `DWD`, schema `DWD-1.0-20180830.xsd`. Media type `application/atsc-dwd+xml` (Annex
H.6). Schedules NRT-content broadcast windows.

#### Table 7.7 — Distribution Window Description (DWD) Semantics
_§7.1.9, A/331:2025-06 p.76-77_

| Element / Attribute | Use | Data Type | Description |
|---|---|---|---|
| `DWD` | 1 | | Root; contains one or more `DistributionWindow`. |
| `DistributionWindow` | 1..N | | One NRT broadcast time window. |
| `@startTime` / `@endTime` | 1 each | dateTime | Window start/end. |
| `@lctTSIRef` | 1 | list of unsignedInt | TSI(s) of the LCT channel(s) carrying the NRT files during this window. |
| `@contentLabel` | 0..1 | unsignedInt | Label shared by windows delivering the same file set; combined with `AppContextId` for app-associated files must be unique per (BSID, label, AppContextId) tuple across the DWD. |
| `AppContextId` | 0..N | anyURI | Application Context Identifier for app-consumable NRT files; required when the content may be consumed by a Broadcaster Application. |
| `@dwFilterCode` | 0..1 | list of unsignedInt | App-specific filter codes scoped to the parent `AppContextId`. |

### 4.7 Regional Service Availability Table (RSAT)

§7.1.10 defers RSAT entirely to ATSC A/200 ("Regional Service Availability") — not transcribed
here; A/200 is not vendored in this repo.

## 5. Media type registrations — Annex H

#### Annex H media types
_A/331:2025-06 p.232-238_

| Fragment | Media type | File ext. | Defined at |
|---|---|---|---|
| S-TSID | `application/route-s-tsid+xml` | `.sls` | §7.1.4 |
| USD for ROUTE | `application/route-usd+xml` | `.rusd` | §7.1.3 |
| USD for MMTP | `application/mmt-usd+xml` | `.musd` | §7.2.1 |
| APD | `application/route-apd+xml` | `.rapd` | §7.1.7 |
| HELD | `application/atsc-held+xml` | `.held` | §7.1.8 |
| DWD | `application/atsc-dwd+xml` | (unspecified in the transcribed excerpt) | §7.1.9 |

All are XML media types constrained to UTF-8 (RFC 7303 §9.1 encoding rules); none provide
confidentiality on their own (rely on the transport / A/360 signing).

## Overlap with other workspace crates

- **`dvb-flute`** — the LCT-channel/TSI/TOI concepts `S-TSID.RS.LS` describes are the same
  RFC 5651 concepts `dvb-flute`'s `LctHeader` models; see [`a331-route.md`](a331-route.md) §0.
- **`transmux`** — the MPD fragment (§4.3) is DASH-IF's MPD verbatim; `transmux` already parses
  DASH MPDs for its DASH-pull/DASH-mux paths. The new `atsc3` crate's job is recognizing *that*
  an SLS bundle carries an MPD and handing it off, not re-implementing MPD parsing.
- **Issue #755 (DVB-MABR)** — ETSI TS 103 769 has its own service-list/session-signalling
  layer (a DVB analogue of SLT/S-TSID) over the same FLUTE/ALC base; no XML-schema-level reuse
  is expected between the two (different XML namespaces/schemas entirely), but both should sit
  on `dvb-flute` for the underlying transport framing.

## Could not establish (see also `README.md`)

- Annex C ("Filtering for Signaling Fragments" — the exact TOI-encoding rule referenced by
  §7.1.6.2) was not read/transcribed in this pass.
- §6.4 (System Time fragment), §6.5 (AEAT), §6.6 (OnscreenMessageNotification), §6.7
  (SignedMultiTable), §6.8 (UserDefined), Annex F (RRT) were not transcribed — flagged inline
  above as deferred, not fabricated.
- The `.xsd` schema files A/331 references throughout (e.g. `SLT-1.0-20211209.xsd`,
  `S-TSID-1.0-20230714.xsd`) are distributed by ATSC as a separate zip, not embedded in the PDF
  text; this document is built from the *informative* field tables + normative prose only, not
  the schemas themselves.
