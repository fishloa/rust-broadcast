# atsc3-route 0.1.0

**Release date:** 2026-08-10

Initial release. `atsc3-route` is split out of the `atsc3` crate (issue #943): `atsc3` keeps the signalling half (LLS + SLS XML), and this crate is the binary ROUTE delta over `rmt-flute`'s RFC 5651 (LCT) / RFC 5775 (ALC) / RFC 6726 (FLUTE) implementation, built to ATSC A/331 Annex A. `#![no_std]` + `alloc`, depending only on `rmt-flute` and `broadcast-common`, so a `no_std` ROUTE receiver doesn't have to pull in an XML stack. Not yet published — no `atsc3-route` version exists on crates.io and no `atsc3-route-v*` tag exists yet.

## What's in it

- `RoutePacket` — a composed ROUTE ALC/LCT packet: an `rmt_flute::LctHeader` validated against A/331 Annex A's mandated field widths (§A.3.4/§A.3.6 — `V`=1, `C`=00, `S`=1, `O`=01, `H`=0, and the PSI/SPI rule for source vs. repair packets), the SPI-bit-dispatched `RouteFecPayloadId`, and the opaque delivery-object payload. Implements `broadcast_common::Parse`/`Serialize`.
- `ExtRoutePresentationTime` — `EXT_ROUTE_PRESENTATION_TIME` (HET 66, §A.3.7.1): the full 64-bit NTP presentation time of an MDE Random Access Point.
- `ExtTol` — `EXT_TOL` (HET 194 fixed-length / 67 variable-length, §A.3.8.1): the delivery object's post-content-encoding transfer length, in its 24-bit (`Bits24`) and 48-bit (`Bits48`) forms.
- `SourceFecPayloadId` / `RepairFecPayloadId` / `RouteFecPayloadId` — the two ROUTE FEC Payload ID layouts (§A.3.5.1/§A.3.5.2): Compact No-Code `start_offset` for source flows, RaptorQ `SBN`/`ESI` (RFC 6330 §3.2) for repair flows, dispatched by the packet's LCT SPI bit.
- `Codepoint` / `FormatId` / `FragMode` / `CodepointSemantics` — the LCT Codepoint field's ROUTE-defined delivery-object semantics (§A.3.6, Table A.3.6), including `Codepoint::known_semantics()` for the directly resolvable `CP` values 1-9.
- Real-fixture tests (`tests/fixture_route.rs`) against `fixtures/atsc3/route-*.bin`: parse, decoded-field assertions, byte-exact round trip, and mutation bite-proofs.
- Two runnable examples: `parse_route_fdt` and `build_route_media_fragment`.
- `serde` feature for `serde::Serialize` derives on public types.
- `tests/label_coverage.rs` / `tests/non_exhaustive_coverage.rs` drift guards (issues #204/#806).

## Out of scope

Same as `rmt-flute`: the FDT/S-TSID/USBD/MPD **XML** documents carried as opaque ROUTE payload bytes (that's `atsc3`'s job, or a consumer's own XML layer), and the RaptorQ encode/decode procedure itself (RFC 6330 is not vendored in this repository).

## Known gap: repair-flow coverage

`RepairFecPayloadId`'s RaptorQ FEC Payload ID layout has **no real-capture corroboration**. Every packet in the real ROUTE capture fixtures this crate was verified against (`fixtures/atsc3/route-*.bin`, 8,885 frames scanned across both source `.pcap` files) ran with the LCT SPI bit set to source-data — no FEC-repair flow was ever active in that capture. The layout is implemented directly from A/331's own Figure A.3.4 bit-diagram (SBN 8 bits / ESI 24 bits, matching RFC 6330 §3.2 — corrected 2026-08-09 in `atsc3/docs/a331-route.md` after the transcription previously read SBN 16 / ESI 16) and is unit-tested only against hand-built vectors. See `fixtures/atsc3/PROVENANCE.md`'s "What was not obtained" section and the `fec.rs` module docs.

## Migration

New crate — no migration needed. MSRV is **1.95.0**.
