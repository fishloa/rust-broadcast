# dvb-subtitle

DVB bitmap subtitling segment parser and serializer, per ETSI EN 300 743 V1.6.1.

[![crates.io](https://img.shields.io/crates/v/dvb-subtitle.svg)](https://crates.io/crates/dvb-subtitle)
[![docs.rs](https://docs.rs/dvb-subtitle/badge.svg)](https://docs.rs/dvb-subtitle)
[![licence](https://img.shields.io/crates/l/dvb-subtitle.svg)](#licence)

## What it does / what it does not

**Does.** Takes the reassembled PES data field of a DVB subtitle stream (the
payload of a PES packet carrying `data_identifier` 0x20) and returns typed
segments: page, region, CLUT, object, display definition, disparity
signalling, alternative CLUT, end of display set and stuffing. Every segment
serializes back to the same bytes. Typical users are subtitle extractors,
stream analysers and head-end tooling.

**Does not.**
- It does not render. It decodes 2/4/8-bit pixel-data run codes into runs
  (`PixelDataSubBlock::runs`), but never composes a page, applies a CLUT to
  produce RGBA, or draws text from character codes.
- It does not inflate the zlib data of a progressive pixel block
  (`ProgressivePixelBlock::compressed_data` stays opaque).
- It does not convert DVB bitmap subtitles to text or WebVTT/SRT (no OCR);
  `caption-convert` documents that as a gap.
- It does not demultiplex TS or reassemble PES; use `mpeg-ts` and `mpeg-pes`.
- It does not parse the PMT `subtitling_descriptor`; that lives in `dvb-si`.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| ETSI EN 300 743, Digital Video Broadcasting (DVB); Subtitling systems | V1.6.1 (2018-10), checked against `private/specs-md/INDEX.md` | §7.1 PES data field (`data_identifier` 0x20, `subtitle_stream_id` 0x00, end marker 0xFF) | Full | Trailing bytes after the last segment are kept for byte-exact round-trip |
| | | §7.2.1 display definition (0x14), §7.2.2 page composition (0x10), §7.2.3 region composition (0x11), §7.2.4 CLUT definition (0x12), §7.2.6 end of display set (0x80), Table 7 stuffing (0xFF) | Full | |
| | | §7.2.5 object data (0x13): 2/4/8-bit pixel-data sub-blocks, map tables, character strings, progressive pixel block header | Partial | Progressive pixel data is typed-opaque (compressed bytes); pixel runs are decoded on request, no rendering |
| | | §7.2.7 disparity signalling (0x15), §7.2.8 alternative CLUT (0x16) | Full | |
| | | Rendering model (§8, §9 decoder behaviour), PMT subtitling descriptor | Not implemented | Descriptor is in `dvb-si` |

Segment types with no typed implementation parse as `AnySegment::Unknown`;
a recognised type whose typed parse fails becomes `AnySegment::Malformed`.
Both re-serialize their body verbatim, so a capture never fails to round-trip
because of one bad segment.

## Feature flags

| Feature | Default | Effect |
|---|---|---|
| `std` | yes | Links `std` (`broadcast-common/std`, `thiserror/std`). Without it the crate is `#![no_std]` + `alloc`. |
| `serde` | no | `serde::Serialize` on segments (serialize-only, camelCase external tagging). |

## Quick start

```rust
use broadcast_common::Parse;
use dvb_subtitle::{PesDataField, DATA_IDENTIFIER, SYNC_BYTE, END_OF_PES_MARKER};

let bytes = [
    DATA_IDENTIFIER,               // 0x20
    0x00,                         // subtitle_stream_id
    SYNC_BYTE,                     // 0x0F
    0x80, 0x00, 0x01, 0x00, 0x00, // end_of_display_set
    END_OF_PES_MARKER,               // 0xFF
];
let field = PesDataField::parse(&bytes).unwrap();
assert_eq!(field.segments.len(), 1);
```

Two runnable examples ship in `examples/` (`parse_segment`, `parse_full_pes`);
run with `cargo run -p dvb-subtitle --example <name>`.

## What is implemented

- **Container:** `PesDataField` (data identifier, stream id, segment list, end
  marker) and the dispatch enum `AnySegment` (9 typed variants plus `Unknown`
  and `Malformed`; `AnySegment::DISPATCHED_SEGMENT_TYPES` lists the codes).
- **Composition:** `PageCompositionSegment` (with `PageState`),
  `RegionCompositionSegment` (`RegionDepth`, `RegionLevelOfCompatibility`,
  `ObjectType`, `ObjectProviderFlag`), `DisplayDefinitionSegment`,
  `EndOfDisplaySetSegment`, `StuffingSegment`.
- **Colour:** `ClutDefinitionSegment`/`ClutEntry` (full and reduced-range
  entries), `AlternativeClutSegment` (`DynamicRangeColourGamut`,
  `OutputBitDepth`).
- **Objects:** `ObjectDataSegment` with `ObjectDataPayload` (interlaced pixel
  sub-blocks, character codes, progressive block, reserved), `PixelDataSubBlock`,
  `PixelRun` iterator.
- **3D:** `DisparitySignallingSegment` (regions, subregions, shift update
  sequences).
- **Errors:** `Error` (`#[non_exhaustive]`), including `FieldOverflow` for a
  value wider than its wire field.

## Design notes

- Zero-copy: parsed structs borrow `&'a [u8]` from the input.
- Symmetric `Parse` / `Serialize`; parse then serialize is byte-identical, and
  serialize then parse compares equal. Tested against real M6 subtitle PES
  reassembled from `fixtures/ts/m6-single.ts` (`tests/it/fixture_subtitle.rs`).
- Serializers reject a value wider than its bit-field (`Error::FieldOverflow`)
  instead of masking; reserved code values are carried verbatim as `Reserved(v)`.
- Hostile input never panics (`tests/it/hostility.rs`: truncation at every length,
  mutation of every byte).
- Public enums are `#[non_exhaustive]` and carry `name()` + `Display`.
- `no_std` + `alloc`; MSRV 1.95.0.

## Related crates

Depends on `broadcast-common` only at runtime (`mpeg-pes` is a dev-dependency
for the fixture test). Feed it from `mpeg-ts` -> `mpeg-pes`. `dvb-si` parses the
PMT/SDT descriptors that locate the subtitle PID. `caption-convert` does
text-caption conversion and documents bitmap subtitles as unsupported.

## MSRV and licence

Rust 1.95.0. MIT OR Apache-2.0.
