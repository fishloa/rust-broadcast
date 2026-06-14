# ISO/IEC 13818-6 §11 BIOP — DVB object-carousel profile (per ETSI TR 101 202)

The BIOP (Broadcast Inter-ORB Protocol) message syntax is normatively defined
in ISO/IEC 13818-6 §11, which is a **paid ISO standard and is not vendored**.
Everything below is transcribed from the **vendored** ETSI guideline
`specs/etsi_tr_101_202_v01.02.01_dvb_data_broadcasting_guidelines.pdf`
(TR 101 202 §4.7), which reproduces the full byte-level syntax tables for the
DVB-profiled subset of BIOP and is the authoritative source for this crate's
`carousel::biop` implementation. Section/table/page numbers below are
TR 101 202's. Where TR 101 202 subordinates to the ISO standard, the
**DVB profile** constraints (alias type_ids, big-endian, fixed tags) make the
ambiguous cases inert on-air — see "CDR / alignment" at the bottom.

This layer sits on top of the DSM-CC framing already transcribed in
`iso_13818_6_carousel.md` (DSI / DII / DDB sections + module reassembly). BIOP
messages live inside the **complete modules** that `ModuleReassembler` produces.

## Constants

### Tagged-profile / component tags (32-bit, `profileId_tag` / `componentId_tag`)
_TR 101 202 §4.7.3, Tables 4.5/4.7_

| Constant | Value | Meaning |
|---|---|---|
| `TAG_BIOP` | `0x49534F06` | BIOP Profile Body |
| `TAG_LITE_OPTIONS` | `0x49534F05` | Lite Options Profile Body |
| `TAG_ObjectLocation` | `0x49534F50` | BIOP::ObjectLocation component |
| `TAG_ConnBinder` | `0x49534F40` | DSM::ConnBinder component |
| `TAG_ServiceLocation` | `0x49534F46` | DSM::ServiceLocation component |

### U-U object type_id aliases (`objectKind` / IOR `type_id`)
_TR 101 202 §4.7.3.1, Table 4.4 — DVB uses ONLY the 3-char alias (+ NUL = 4 bytes)_

| Full type_id | alias | `objectKind_data` (4 bytes) |
|---|---|---|
| `DSM::Directory` | `"dir"` | `0x64697200` |
| `DSM::File` | `"fil"` | `0x66696C00` |
| `DSM::Stream` | `"str"` | `0x73747200` |
| `DSM::ServiceGateway` | `"srg"` | `0x73726700` |
| `BIOP::StreamEvent` | `"ste"` | `0x73746500` |

### Tap `use` values
_TR 101 202 §4.7.3.2, Table 4.6_

| Constant | Value | Broadcast on PID |
|---|---|---|
| `BIOP_DELIVERY_PARA_USE` | `0x0016` | Module delivery parameters |
| `BIOP_OBJECT_USE` | `0x0017` | BIOP objects in Modules |

### Binding type (`bindingType`, 8-bit)
_TR 101 202 §4.7.4.1, Table 4.9_

| Value | Meaning |
|---|---|
| `0x01` | `nobject` — name bound to a non-Directory/ServiceGateway object |
| `0x02` | `ncontext` — name bound to a Directory or ServiceGateway |

(`composite` is not supported for U-U object carousels.)

### Message header constants (all four message types)

| Field | Value |
|---|---|
| `magic` (4 bytes) | `0x42494F50` (`"BIOP"`) |
| `biop_version.major` | `0x01` |
| `biop_version.minor` | `0x00` |
| `byte_order` | `0x00` (big-endian — DVB mandatory) |
| `message_type` | `0x00` |

---

## Table 4.3 — IOP::IOR syntax
_§4.7.3.1, PDF p. 30_

| Syntax | bits | Value | Comment |
|---|---|---|---|
| `type_id_length` | 32 | N1 | |
| `type_id_byte` × N1 | 8 each | + | see Table 4.4 (DVB: a 3-char alias + NUL ⇒ N1 = 4) |
| `alignment_gap` × (4−(N1%4)) | 8 each | `0xFF` | **only if** `N1 % 4 ≠ 0` — CDR alignment. Never present for DVB alias type_ids (N1=4). |
| `taggedProfiles_count` | 32 | N2 | ≥ 1; first profile is TAG_BIOP or TAG_LITE_OPTIONS |
| per profile: `profileId_tag` | 32 | + | e.g. TAG_BIOP / TAG_LITE_OPTIONS |
| per profile: `profile_data_length` | 32 | N3 | |
| per profile: `profile_data_byte` × N3 | 8 each | | e.g. a BIOPProfileBody / LiteOptionsProfileBody |

DVB guideline: only alias type_ids are used (so no alignment stuffing). Receivers
must process at least the first profile body; others may be ignored.

## Table 4.5 — BIOP Profile Body syntax
_§4.7.3.2, PDF p. 32_

| Syntax | bits | Value | Comment |
|---|---|---|---|
| `profileId_tag` | 32 | `0x49534F06` | TAG_BIOP |
| `profile_data_length` | 32 | * | |
| `profile_data_byte_order` | 8 | `0x00` | big-endian |
| `liteComponents_count` | 8 | N1 | |
| **BIOP::ObjectLocation {** | | | (1st component, mandatory) |
| `componentId_tag` | 32 | `0x49534F50` | TAG_ObjectLocation |
| `component_data_length` | 8 | * | |
| `carouselId` | 32 | + | |
| `moduleId` | 16 | + | |
| `version.major` | 8 | `0x01` | |
| `version.minor` | 8 | `0x00` | |
| `objectKey_length` | 8 | N2 | ≤ `0x04` (DVB) |
| `objectKey_data_byte` × N2 | 8 each | + | |
| **}** | | | |
| **DSM::ConnBinder {** | | | (2nd component, mandatory) |
| `componentId_tag` | 32 | `0x49534F40` | TAG_ConnBinder |
| `component_data_length` | 8 | * | |
| `taps_count` | 8 | N3 | |
| first BIOP::Tap: `id` | 16 | `0x0000` | user private |
| first BIOP::Tap: `use` | 16 | `0x0016` | BIOP_DELIVERY_PARA_USE |
| first BIOP::Tap: `association_tag` | 16 | + | |
| first BIOP::Tap: `selector_length` | 8 | `0x0A` | |
| first BIOP::Tap: `selector_type` | 16 | `0x0001` | MESSAGE |
| first BIOP::Tap: `transactionId` | 32 | * | transactionId of the DII carrying the module |
| first BIOP::Tap: `timeout` | 32 | * | µs |
| then (N3−1)× BIOP::Tap: `id` | 16 | `0x0000` | |
| `use` | 16 | + | |
| `association_tag` | 16 | + | |
| `selector_length` | 8 | N4 | |
| `selector_data_byte` × N4 | 8 each | | |
| **}** | | | |
| then (N5 = N1−2)× BIOP::LiteComponent: `componentId_tag` | 32 | + | |
| `component_data_length` | 8 | N6 | |
| `component_data_byte` × N6 | 8 each | | |

DVB guidelines: `byte_order = 0x00`; the first two components are exactly one
ObjectLocation then one ConnBinder, in that order; `objectKey_length ≤ 0x04`;
the BIOP Profile Body refers only to objects in the same carousel (its
`carouselId` equals the carousel's); if a BIOP_DELIVERY_PARA_USE tap is present
it is the first tap in the ConnBinder; the `id` field is 0 if unused.

## Table 4.7 — Lite Options Profile Body (with ServiceLocation) syntax
_§4.7.3.3, PDF p. 34_

| Syntax | bits | Value | Comment |
|---|---|---|---|
| `profileId_tag` | 32 | `0x49534F05` | TAG_LITE_OPTIONS |
| `profile_data_length` | 32 | * | |
| `profile_data_byte_order` | 8 | `0x00` | big-endian |
| `component_count` | 8 | N1 | |
| **DSM::ServiceLocation {** | | | (must be the first component) |
| `componentId_tag` | 32 | `0x49534F46` | TAG_ServiceLocation |
| `component_data_length` | 32 | * | |
| `serviceDomain_length` | 8 | `0x14` | 20 — length of the carousel NSAP address |
| `serviceDomain_data()` | 160 | + | DVBcarouselNSAPaddress (Table 4.8) |
| **CosNaming::Name() {** | | | pathName |
| `nameComponents_count` | 32 | N2 | |
| per component: `id_length` | 32 | N3 | |
| `id_data_byte` × N3 | 8 each | + | |
| per component: `kind_length` | 32 | N4 | |
| `kind_data_byte` × N4 | 8 each | + | as type_id (Table 4.4) |
| **}** | | | |
| `initialContext_length` | 32 | N5 | |
| `InitialContext_data_byte` × N5 | 8 each | | |
| **}** | | | |
| then (N6 = N1−1)× BIOP::LiteOptionComponent: `componentId_tag` | 32 | + | |
| `component_data_length` | 8 | N7 | |
| `component_data_byte` × N7 | 8 each | | |

## Table 4.8 — DVB Carousel NSAP Address syntax (20 bytes)
_§4.7.3.4, PDF p. 35_

| Syntax | bits | Value | Comment |
|---|---|---|---|
| `AFI` | 8 | `0x00` | NSAP for private use |
| `Type` | 8 | `0x00` | Object carousel NSAP address |
| `carouselId` | 32 | + | |
| `specifierType` | 8 | `0x01` | IEEE OUI |
| `specifierData` (IEEE OUI) | 24 | `0x00_00_DVB`* | constant for the DVB OUI |
| `transport_stream_id` | 16 | + | |
| `original_network_id` | 16 | + | |
| `service_id` | 16 | + | = MPEG-2 program_number |
| `reserved` | 32 | `0xFFFFFFFF` | |

\* the DVB OUI value; semantics per EN 301 192. Total = 8+8+32+8+24+16+16+16+32 = 160 bits = 20 bytes (matches `serviceDomain_length = 0x14`).

## Table 4.9 — BIOP::DirectoryMessage syntax
_§4.7.4.1, PDF p. 36 — ServiceGateway is identical except `objectKind = "srg"` (§4.7.4.4)_

| Syntax | bits | Value | Comment |
|---|---|---|---|
| `magic` | 4×8 | `0x42494F50` | "BIOP" |
| `biop_version.major` | 8 | `0x01` | |
| `biop_version.minor` | 8 | `0x00` | |
| `byte_order` | 8 | `0x00` | big-endian |
| `message_type` | 8 | `0x00` | |
| `message_size` | 32 | * | bytes following this field |
| `objectKey_length` | 8 | N1 | ≤ 0x04 |
| `objectKey_data_byte` × N1 | 8 each | + | |
| `objectKind_length` | 32 | `0x00000004` | |
| `objectKind_data` | 4×8 | `0x64697200` | "dir" (or `0x73726700` "srg" for ServiceGateway) |
| `objectInfo_length` | 16 | N2 | |
| `objectInfo_data_byte` × N2 | 8 each | + | |
| `serviceContextList_count` | 8 | N3 | |
| per context: `context_id` | 32 | | |
| per context: `context_data_length` | 16 | N9 | |
| per context: `context_data_byte` × N9 | 8 each | + | |
| `messageBody_length` | 32 | * | |
| `bindings_count` | 16 | N4 | |
| **per binding: BIOP::Name {** | | | |
| `nameComponents_count` | 8 | N5 | DVB: = 1 |
| per name-comp: `id_length` | 8 | N6 | |
| `id_data_byte` × N6 | 8 each | + | |
| per name-comp: `kind_length` | 8 | N7 | |
| `kind_data_byte` × N7 | 8 each | + | as type_id (Table 4.4) |
| **}** | | | |
| `bindingType` | 8 | + | `0x01` nobject / `0x02` ncontext |
| `IOP::IOR()` | | + | objectRef (Table 4.3) |
| `objectInfo_length` | 16 | N8 | |
| `objectInfo_data_byte` × N8 | 8 each | + | |

Strings are NUL-terminated (`0x00`). DVB: `nameComponents_count = 1`; receivers
must skip over `serviceContextList` and `objectInfo`.

## Table 4.10 — BIOP::FileMessage syntax
_§4.7.4.2, PDF p. 38_

| Syntax | bits | Value | Comment |
|---|---|---|---|
| `magic` | 4×8 | `0x42494F50` | "BIOP" |
| `biop_version.major` | 8 | `0x01` | |
| `biop_version.minor` | 8 | `0x00` | |
| `byte_order` | 8 | `0x00` | big-endian |
| `message_type` | 8 | `0x00` | |
| `message_size` | 32 | * | |
| `objectKey_length` | 8 | N1 | ≤ 0x04 |
| `objectKey_data_byte` × N1 | 8 each | + | |
| `objectKind_length` | 32 | `0x00000004` | |
| `objectKind_data` | 4×8 | `0x66696C00` | "fil" |
| `objectInfo_length` | 16 | N2 | |
| `DSM::File::ContentSize` | 64 | + | first 8 bytes of objectInfo |
| `objectInfo_data_byte` × (N2−8) | 8 each | + | |
| `serviceContextList_count` | 8 | N3 | |
| per context: `context_id` | 32 | | |
| per context: `context_data_length` | 16 | N9 | |
| per context: `context_data_byte` × N9 | 8 each | + | |
| `messageBody_length` | 32 | * | |
| `content_length` | 32 | N4 | |
| `content_data_byte` × N4 | 8 each | + | actual file content |

Note: `objectInfo_length` (N2) is ≥ 8 because `DSM::File::ContentSize` (8 bytes)
is the leading part of objectInfo.

## Table 4.14 — BIOP::ModuleInfo syntax (the DII `moduleInfoBytes`)
_§4.7.5.1, PDF p. 42_

| Syntax | bits | Value | Comment |
|---|---|---|---|
| `ModuleTimeOut` | 32 | + | µs to time out acquisition of all blocks |
| `BlockTimeOut` | 32 | + | µs to time out the next block |
| `MinBlockTime` | 32 | + | min µs between two blocks |
| `taps_count` | 8 | N1 | ≥ 1 (≥ one BIOP_OBJECT_USE tap) |
| per tap: `Id` | 16 | `0x0000` | user private |
| per tap: `Use` | 16 | `0x0017` | BIOP_OBJECT_USE |
| per tap: `association_tag` | 16 | + | ES on which the modules are broadcast |
| per tap: `selector_length` | 8 | `0x00` | (zero-length selector) |
| `UserInfoLength` | 8 | N2 | |
| `userInfo_data_byte` × N2 | 8 each | + | descriptor loop (incl. NUL terminators) |

The `userInfo` loop carries Data-Carousel module descriptors. DVB receivers must
support the **`compressed_module_descriptor` (tag `0x09`)**, which signals that
the module is transmitted zlib-compressed (see below).

## Table 4.15 — BIOP::ServiceGatewayInfo syntax (the DSI `privateData`)
_§4.7.5.2, PDF p. 43 — carried in the DownloadServerInitiate `privateDataByte`_

| Syntax | bits | Value | Comment |
|---|---|---|---|
| `IOP::IOR()` | | + | IOR of the ServiceGateway (Table 4.3) |
| `downloadTaps_count` | 8 | N1 | software-download Taps |
| `Tap()` × N1 | | + | semantics not defined by TR 101 202 (parse-to-raw) |
| `serviceContextList_count` | 8 | N2 | |
| per context: `context_id` | 32 | | |
| per context: `context_data_length` | 16 | N9 | |
| per context: `context_data_byte` × N9 | 8 each | + | |
| `userInfoLength` | 16 | N3 | |
| `userInfo_data_byte` × N3 | 8 each | + | descriptor loop |

In the DSI, the `serverId` is 20 bytes of `0xFF`, the `compatibilityDescriptor()`
is zero-length, and `privateDataLength` gives the byte count of this structure.
The `userInfo` field is a DVB/private descriptor loop. The `downloadTaps`/
`serviceContextList` semantics are not defined by TR 101 202, so they are parsed
to raw bytes (in practice `downloadTaps_count` is typically 0).

---

## compressed_module_descriptor (tag 0x09)
_TR 101 202 §4.6.6.10, PDF p. 20 — appears in the ModuleInfo `userInfo` loop_

Standard DVB descriptor framing (`descriptor_tag` 8, `descriptor_length` 8) then
the body. The DVB guideline is that the module bytes are zlib-compressed; the
zlib payload structure (RFC 1951 DEFLATE wrapped per RFC 1950) is:

| Field | bytes | Comment |
|---|---|---|
| `compression_method` | 1 | zlib CMF (RFC 1950) |
| `flags_check` | 1 | zlib FLG |
| `compressed_data` | n | DEFLATE stream (RFC 1951) |
| `check_value` | 4 | Adler-32 |

Decompression is gated behind the optional **`flate2`** feature (off by default);
without it the compressed module bytes are exposed raw.

## CDR / alignment — the one bounded caveat
_§4.7.3.1, PDF pp. 30–31_

BIOP uses CDR-Lite encoding (ISO/IEC 13818-6 §11, citing OMG CORBA CDR). The only
alignment rule that surfaces in these tables is the `alignment_gap` in
Table 4.3, taken `if (type_id_length % 4 ≠ 0)`. TR 101 202's DVB guideline
mandates **alias type_ids only** — always 3 chars + NUL = 4 bytes — so
`N1 % 4 == 0` always and the gap is **always zero bytes** in a conformant DVB
stream. The implementation therefore parses the IOR with no alignment gap and
**rejects** a non-alias `type_id_length` (`N1 % 4 ≠ 0`) as unsupported rather
than guessing the ISO alignment rule. The `*_byte_order` fields are the CDR
byte-order flag and are fixed at `0x00` (big-endian) by DVB guideline.

## Table 4.12 — Tap use values for Stream / StreamEvent messages
_§4.7.4.3, PDF p. 40_

| Constant | Value | Broadcast on PID |
|---|---|---|
| `STR_NPT_USE` | `0x000B` | Stream NPT descriptors |
| `STR_STATUS_AND_EVENT_USE` | `0x000C` | Stream mode + stream event descriptors |
| `STR_EVENT_USE` | `0x000D` | Stream event descriptors |
| `STR_STATUS_USE` | `0x000E` | Stream mode descriptors |
| `BIOP_ES_USE` | `0x0018` | Elementary stream (video/audio) |
| `BIOP_PROGRAM_USE` | `0x0019` | Program (DVB service) reference |

## Table 4.11 — BIOP::StreamMessage syntax
_§4.7.4.3, PDF p. 39 — `objectKind = "str"`_

| Syntax | bits | Value | Comment |
|---|---|---|---|
| common header (magic..message_size) | | | as Table 4.9 |
| `objectKey_length` | 8 | N1 | |
| `objectKey_data_byte` × N1 | 8 each | + | |
| `objectKind_length` | 32 | `0x00000004` | |
| `objectKind_data` | 4×8 | `0x73747200` | "str\0" |
| `objectInfo_length` | 16 | N6 | |
| **DSM::Stream::Info_T {** | | | objectInfo head |
| `aDescription_length` | 8 | N2 | |
| `aDescription_bytes` × N2 | 8 each | + | |
| `duration.aSeconds` | 32 | + | AppNPT seconds (**signed**, `simsbf`) |
| `duration.aMicroSeconds` | 16 | + | AppNPT microseconds |
| `audio` | 8 | + | |
| `video` | 8 | + | |
| `data` | 8 | + | |
| **}** | | | (Info_T = N2 + 10 bytes) |
| `objectInfo_byte` × (N6 − (N2+10)) | 8 each | + | trailing objectInfo |
| `serviceContextList_count` | 8 | N3 | + loop (context_id 32, context_data_length 16, data) |
| `messageBody_length` | 32 | * | |
| `taps_count` | 8 | N4 | |
| per tap: `id` | 16 | `0x0000` | |
| per tap: `use` | 16 | + | Table 4.12 (ES_USE / PROGRAM_USE / STR_*_USE) |
| per tap: `association_tag` | 16 | + | |
| per tap: `selector_length` | 8 | `0x00` | no selector |

## Table 4.13 — BIOP::StreamEventMessage syntax
_§4.7.4.5, PDF p. 41 — `objectKind = "ste"`_

Identical to StreamMessage through the `DSM::Stream::Info_T` block, then adds an
event-name list, and ends with an `eventId` list after the tap loop:

| Syntax | bits | Value | Comment |
|---|---|---|---|
| common header + objectKey + objectKind | | `0x73746500` | "ste\0" |
| `objectInfo_length` | 16 | N6 | |
| `DSM::Stream::Info_T { … }` | | | exactly as Table 4.11 (N2 + 10 bytes) |
| **DSM::Event::EventList_T {** | | | |
| `eventNames_count` | 16 | N3 | |
| per name: `eventName_length` | 8 | N4 | |
| `eventName_data_byte` × N4 | 8 each | + | NUL-terminated |
| **}** | | | |
| `objectInfo_byte` × (N6 − (N2+10) − (2 + ΣeventName)) | 8 each | + | trailing objectInfo |
| `serviceContextList_count` | 8 | N | + loop |
| `messageBody_length` | 32 | * | |
| `taps_count` | 8 | N5 | tap = id(16)/use(16, Table 4.12)/association_tag(16)/selector_length(8)=0 |
| `eventIds_count` | 8 | N3 | = `eventNames_count` |
| `eventId` × N3 | 16 each | + | correlates to the event names |

DVB note: the eventId sequence count equals the eventNames count. DSM-CC events
are **not** DVB-SI events.

## Table 4.17 — carousel_identifier_descriptor (tag 0x13)
_§4.7.7.1, PDF p. 45 — inserted in the PMT 2nd (ES_info) descriptor loop of the DSI's elementary stream_

| Syntax | bits | Value | Comment |
|---|---|---|---|
| `descriptor_tag` | 8 | `0x13` | |
| `descriptor_length` | 8 | * | |
| `carousel_id` | 32 | + | = the object carousel's carouselId |
| `FormatId` | 8 | + | identifies the FormatSpecifier — Table 4.17a |
| `FormatSpecifier()` | 8×N2 | + | length depends on `FormatId` (Table 4.17a) |
| `private_data_byte` × N1 | 8 each | + | remainder of the descriptor |

`FormatId` selects the FormatSpecifier layout (Table 4.17a): `0x00` = none (no
FormatSpecifier bytes); `0x01` = the aggregated ServiceGateway-location fields
(see Table 4.17a below); `0x02–0x7F` reserved; `0x80–0xFF` private.

## Table 4.17a — FormatSpecifier in the carousel_identifier_descriptor
_§4.7.7.1, PDF p. 46 — bit-widths read from the page-46 PDF render (the
`pdftotext` bits column was misaligned; verified against the rendered table)._

| FormatId | FormatSpecifier | Notes |
|---|---|---|
| `0x00` | (none) | ServiceGateway located only via the DSI/DII messages |
| `0x01` | the aggregated ServiceGateway-location fields below | all `uimsbf` |
| `0x02`–`0x7F` | reserved (DVB) | |
| `0x80`–`0xFF` | reserved (private) | |

`FormatId = 0x01` FormatSpecifier (16 bytes fixed + ObjectKeyData):

| Field | bits | Comment |
|---|---|---|
| `ModuleVersion` | 8 | |
| `ModuleId` | 16 | |
| `BlockSize` | 16 | |
| `ModuleSize` | 32 | |
| `CompressionMethod` | 8 | |
| `OriginalSize` | 32 | |
| `TimeOut` | 8 | seconds — **NB:** TR 101 202 v1.2.1 shows 8 bits here; the later canonical carousel_identifier_descriptor (TS 102 809 / EN 301 192) uses a 32-bit TimeOut. This crate follows the vendored TR 101 202 (8-bit). |
| `ObjectKeyLength` | 8 | N1 |
| `ObjectKeyData` × N1 | 8 each | object key of the ServiceGateway object |
