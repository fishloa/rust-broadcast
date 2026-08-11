# Changelog

All notable changes to `atsc3-route` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Removed
- **`ExtRoutePresentationTime`** and **`ExtTol`** typed decoders removed from the
  public API. No publicly-available ATSC 3.0 ROUTE capture contains either
  extension (14,000+ real packets from three independent sources scanned, zero
  hits). This crate's fixture discipline requires every implemented type to be
  exercised by a byte-exact round-trip against a real capture. The HET constants
  (`HET_EXT_ROUTE_PRESENTATION_TIME`, `HET_EXT_TOL_24`, `HET_EXT_TOL_48`)
  remain for callers walking extension chains. The typed decoders will be
  re-added when a real capture containing these extensions surfaces.

## [0.1.0] - 2026-08-11

Split out of `atsc3` (issue #943): `atsc3` keeps the signalling half (LLS +
SLS XML), this crate is the binary ROUTE delta over `rmt-flute`'s RFC
5651/5775/6726 LCT/ALC/FLUTE implementation. Not yet published — no
`atsc3-route` version exists on crates.io and no `atsc3-route-v*` tag exists.

### Changed
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.

### Added

- `RoutePacket` — a composed ROUTE ALC/LCT packet: an `rmt_flute::LctHeader`
  validated against A/331 Annex A's mandated field widths (§A.3.4/§A.3.6 —
  `V`=1, `C`=00, `S`=1, `O`=01, `H`=0, and the PSI/SPI rule for source vs.
  repair packets), the SPI-bit-dispatched `RouteFecPayloadId`, and the opaque
  delivery-object payload. Implements `broadcast_common::Parse`/`Serialize`.
- `ExtRoutePresentationTime` — `EXT_ROUTE_PRESENTATION_TIME` (HET 66,
  §A.3.7.1): the full 64-bit NTP presentation time of an MDE Random Access
  Point.
- `ExtTol` — `EXT_TOL` (HET 194 fixed-length / 67 variable-length,
  §A.3.8.1): the delivery object's post-content-encoding transfer length, in
  its 24-bit (`Bits24`) and 48-bit (`Bits48`) forms.
- `SourceFecPayloadId` / `RepairFecPayloadId` / `RouteFecPayloadId` — the two
  ROUTE FEC Payload ID layouts (§A.3.5.1/§A.3.5.2): Compact No-Code
  `start_offset` for source flows, RaptorQ `SBN`/`ESI` (RFC 6330 §3.2) for
  repair flows, dispatched by the packet's LCT SPI bit.
- `Codepoint` / `FormatId` / `FragMode` / `CodepointSemantics` — the LCT
  Codepoint field's ROUTE-defined delivery-object semantics (§A.3.6, Table
  A.3.6), including `Codepoint::known_semantics()` for the directly
  resolvable `CP` values 1-9.
- Real-fixture tests (`tests/fixture_route.rs`) against
  `fixtures/atsc3/route-*.bin` (the FDT-Instance packet, both media
  fragments, and a cross-check against the reassembled SLS package's S-TSID
  XML): parse, decoded-field assertions, byte-exact round trip, and mutation
  bite-proofs (HDR_LEN corruption, SPI-bit flip, payload mutation).
- Two runnable examples: `parse_route_fdt` (parse the real FDT-Instance
  fixture, walk its extension chain, decode Codepoint) and
  `build_route_media_fragment` (construct + serialize a media-segment
  fragment matching the real video-fragment fixture's shape).
- `#![no_std]` + `alloc` (via `rmt-flute`); builds with `--no-default-features`.
- `serde` support behind the `serde` feature.
- `tests/label_coverage.rs` / `tests/non_exhaustive_coverage.rs` drift
  guards (issues #204/#806).

### Known gap

- `RepairFecPayloadId`'s RaptorQ FEC Payload ID layout has **no real-capture
  corroboration** — every packet in the fixtures this crate was verified
  against ran with the LCT SPI bit set (source-data), so no repair-flow
  packet exists to test against. Implemented directly from A/331's own
  Figure A.3.4 bit-diagram (SBN 8 bits / ESI 24 bits, matching RFC 6330 §3.2 —
  corrected 2026-08-09 in `atsc3/docs/a331-route.md` after the transcription
  previously read SBN 16 / ESI 16) and unit-tested against hand-built
  vectors only. See the crate root / `fec.rs` module docs.
