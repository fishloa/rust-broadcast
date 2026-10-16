# atsc3

> **ARCHIVED — abandoned, permanently unpublished, no further development.** There is no `atsc3` release on crates.io and none is planned. The ATSC 3.0 market is closed enough that real captures are unobtainable, short of the real-fixture bar every other crate here clears. The crate still compiles and its tests still pass against the one real LLS/SLT fixture it has, but it receives no investment. Do not depend on it.

ATSC 3.0 (NextGen TV) Low-Level Signalling: the A/331 LLS binary envelope and the Service List Table (SLT) it carries.

## What it does / what it does not

It parses and re-serializes the 4-byte `LLS_table()` envelope, optionally gunzips the table body, and parses the SLT XML (the rapid-channel-scan bootstrap, analogous to MPEG-2 PAT) into typed Rust structs. It is useful for reading what services a 3.0 broadcast announces and where their Service Layer Signalling is delivered.

It does **not** implement:

- A/321 bootstrap / physical-layer discovery signalling;
- Service Layer Signalling (USBD, S-TSID, APD, HELD, DWD) or ROUTE/DASH and MMT delivery (the ROUTE binary framing lives in the likewise archived `atsc3-route`);
- the LLS tables other than SLT: RRT, SystemTime, AEAT, onscreen-message notification, CDT, DRCT and UserDefined bodies are identified by `LlsTableId` but their payloads stay opaque (gzip bytes);
- `SignedMultiTable` (LLS id `0xFE`): it is folded into the reserved range;
- the SLT attributes beyond the initial subset (see below). There is no SLT serializer; only the envelope round-trips.

## Standards supported

| Standard | Edition | Scope covered | Status | Notes |
|----------|---------|---------------|--------|-------|
| ATSC A/331 — Signaling, Delivery, Synchronization, and Error Protection | Code and docs cite A/331:2025-06; the vendored transcription is A/331:2026-04. `docs/README.md` records that no transcribed value differs between the two. | §6.2 `LLS_table()` envelope and Table 6.1 table IDs; §6.3 SLT, Table 6.2 (subset), Tables 6.4 / 6.5 (category and SLS-protocol code values) | Partial | SLT subset: `SLT@bsid`; per `Service`: `serviceId`, `majorChannelNo`, `minorChannelNo`, `serviceCategory`, `shortServiceName`, `hidden`, and `BroadcastSvcSignaling` (protocol + versions, destination IP/port, source IP). Not modelled: `globalServiceID`, `sltSvcSeqNum`, `protected`, `hideInGuide`, `SvcInetUrl`, `OtherBsid`/`OtherRf`. Only IPv4 addresses are modelled. |
| ATSC A/323 — Dedicated Return Channel | 2026-04 (not vendored) | The code point `LLS_table_id = 0x07` (DRCT) only | Typed-opaque | Verified against the A/323 PDF fetched from atsc.org during development, not against a vendored copy. |
| ATSC A/360 — Security | not established | `LLS_table_id = 0x06` (CDT) only | Typed-opaque | Unverified against a vendored PDF. |
| IETF RFC 1952 — GZIP file format | May 1996 | Table-body decompression | Full | Via `flate2`, `std` feature only. |

## Feature flags

| Feature | Default | Enables |
|---------|---------|---------|
| `std`   | yes     | Gzip decompression (`flate2`) and XML parsing (`roxmltree/std`); `LlsEnvelope::decompress`. |
| `serde` | no      | `serde::Serialize` / `Deserialize` derives on the public types. |

The envelope parse/serialize is `no_std` + `alloc`; without `std` the compressed body stays an opaque byte slice.

## Quick start

```rust
use atsc3::{LlsEnvelope, LlsTableId, Slt};
use broadcast_common::Parse;

// A real LLS_table() capture: a gzip-compressed SLT with two ROUTE services.
let bytes = test_fixtures::read("atsc3/slt-lls-2019-01-07.bin");
let envelope = LlsEnvelope::parse(&bytes).unwrap();
assert_eq!(envelope.table_id, LlsTableId::Slt);

let xml_bytes = envelope.decompress().unwrap();
let xml = std::str::from_utf8(&xml_bytes).unwrap();
let slt = Slt::parse(xml).unwrap();
assert_eq!(slt.services.len(), 2);
assert_eq!(slt.services[0].short_service_name.as_deref(), Some("NATNL"));
```

## What is implemented

- `LlsEnvelope`: the common header (`LLS_table_id`, `LLS_group_id`, `group_count_minus1`, `LLS_table_version`) plus the borrowed table body, with symmetric `Parse`/`Serialize` and `decompress` (`std`).
- `LlsTableId`: SLT `0x01`, RRT `0x02`, SystemTime `0x03`, AEAT `0x04`, OnscreenMessageNotification `0x05`, CDT `0x06`, DRCT `0x07`, `Reserved(u8)`, UserDefined `0xFF`. (`0x07` was once mis-modelled as CAP; it is the Dedicated Return Channel Table.)
- `Slt`, `SltService`, `BroadcastSvcSignaling`, `ServiceCategory`, `SlsProtocol`: the typed SLT model and its XML parser (`Slt::parse`).

The only real capture available is the single committed fixture `fixtures/atsc3/slt-lls-2019-01-07.bin`, extracted from a third-party open-source library's test suite (provenance in `fixtures/atsc3/PROVENANCE.md`).

## Design notes

- Zero-copy envelope (`&'a [u8]` body), symmetric serialize, byte-exact round-trip against the real fixture; a corrupted-gzip test proves the fixture is actually exercised.
- Public enums are `#[non_exhaustive]` with `name()` / `Display` labels.
- XML is parsed with `roxmltree`, the workspace's one remaining non-`quick-xml` XML dependency, a consequence of the crate being abandoned before migration.
- The crate is `#![forbid(unsafe_code)]`.

## Planned (will not be done)

Retained for context only; none of this is scheduled:

- A/321 bootstrap signalling.
- A/331 ROUTE/DASH object carriage (the LCT/ALC/FLUTE layer exists in `rmt-flute`, and ROUTE framing constants in `atsc3-route`).
- A/331 MMT signalling (§7.2).
- Service Layer Signalling: USBD, S-TSID, APD, HELD, DWD (§7.1).
- The remaining SLT / Service XML attributes listed above.

## Related crates

Depends on `broadcast-common` only. `atsc3-route` (also archived) is the delivery-side sibling and builds on `rmt-flute`. DVB's analogue of this crate's role is `dvb-si`.

## MSRV and licence

Minimum supported Rust version 1.95.0. Licensed under MIT OR Apache-2.0.
