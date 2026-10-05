# dvb-subtitle 0.5.0

_Released 2026-10-05._

Breaking release (0.4 -> 0.5) for ETSI EN 300 743 subtitling. It tightens parsing so a broken typed segment parser can no longer hide behind a byte-exact round trip, removes three stored fields that could disagree with the data they describe, stops serializers from silently truncating or masking over-range values, and fixes one real serialization bug that corrupted object positions. If you construct or destructure `ObjectDataPayload::Characters`, `DisparityShiftUpdateSequence` or `InterlacedPixelsData`, you must change code; if you only parse, you should check how you treat `AnySegment::Unknown` (see below).

## Breaking changes (source)

All three are in segment types you can build or pattern-match as plain structs/variants.

1. **`ObjectDataPayload::Characters` no longer has `number_of_codes`** (#1108 DS-W3). Table 20's count is now derived from `character_codes.len()` on serialize.
   ```rust
   // before (0.4)
   ObjectDataPayload::Characters { number_of_codes: 2, character_codes: vec![0x41, 0x42] }
   // after (0.5)
   ObjectDataPayload::Characters { character_codes: vec![0x41, 0x42] }
   ```
2. **`DisparityShiftUpdateSequence` no longer has `division_period_count`** (#1108 DS-W3). Table 30's count is the length of `intervals`; construct the struct with `interval_duration` and `intervals` only.
3. **`InterlacedPixelsData::stuffing_byte: Option<u8>` is now `stuffing: &'a [u8]`** (#1108 DS-W6). Trailing bytes beyond the first used to be dropped; they are now preserved for a byte-exact round trip. Replace `Some(b)` with `&[b]` and `None` with `&[]`.

`PageCompositionSegment`'s `suffix` was `pub(crate)` in 0.4.0, so removing it (it could never hold anything) is invisible to downstream code.

## Behaviour changes you will notice when parsing

- **Recognised-but-malformed segments are no longer `Unknown`** (#1108 DS-W1). `AnySegment` has a new `Malformed { segment_type, page_id, data, err }` variant. `PesDataField::parse` used to fold a known `segment_type` whose typed parse returned `Err` into the same raw-passthrough `Unknown` that an unimplemented type gets, so a defective typed parser still round-tripped byte-exactly and stayed invisible. `Malformed` re-serializes the raw bytes verbatim, like `Unknown`, and carries the parse error. `AnySegment` is `#[non_exhaustive]`, so this is not a compile break for code that already has a wildcard arm, but any code that treated `Unknown` as "the one place broken data shows up" must now also look at `Malformed`. `AnySegment::name()` returns `"MALFORMED"` for it.
- **`sync_byte` is checked in every segment's `parse`** (#1108 DS-W2), as `Error::BadSyncByte`, not only in `PesDataField`'s segment loop. Feeding a segment parser bytes that do not begin with `0x0F` now fails.
- **Disparity subregion counts are validated** (#1108 DS-W3). `DisparityRegion` must hold 1 to 4 subregions (`Error::InvalidSubregionCount`): Table 29's `number_of_subregions_minus_1` is 2 bits and a larger count was previously wrapped by `as u8 & 0x03`. Every `Subregion`'s `subregion_horizontal_position`/`subregion_width` must be present for all subregions or none, matching the region's count (`Error::SubregionPositionMismatch`).
- **Trailing partial entries are rejected, not dropped** (#1108 DS-W6). `AlternativeClutSegment` and `DisparitySignallingSegment` return `Error::TrailingEntryBytes` (or `Error::BufferTooShort`) when the remainder is not a whole entry; previously integer division or a bare loop `break` discarded it.

## Behaviour changes when serializing

- **Over-range segment bodies are an error, not a wrapped length** (#1108, #1129). Every segment type's `serialize_into` (`object_data`, `region_composition`, `page_composition`, `clut_definition`, `alternative_clut`, `disparity_signalling`, `display_definition`, `stuffing`, and the `Unknown` fallback in `any`) returns `Error::SegmentTooLarge` for a body of 64 KiB or more instead of writing a smaller `segment_length`.
- **`RegionCompositionSegment` object positions** (#1044, #1129). `object_horizontal_position` and `object_vertical_position` are 12-bit fields (Table 11). A value above `0x0FFF` now returns `Error::FieldOverflow` (wraps `broadcast_common::len::FieldOverflow`) instead of being masked.

## Fixes

- **#1044: objects at vertical position 256 or more were written wrong.** The object-entry serializer wrote the top nibble of `object_vertical_position` into the reserved high nibble of byte 4 instead of the low nibble (Table 11: `reserved(4)` then the 12-bit position), silently corrupting the position of any object with `object_vertical_position >= 256`. Streams you re-serialized with 0.4 and earlier may carry wrong positions.

## New API

- `PixelDataSubBlock::runs()` (#1108 DS-W5) returns `Option<PixelRunIter>`: typed `PixelRun { pixel_code: u8, run_length: u32 }` values decoded from a 2/4/8-bit pixel-data code string (Tables 22-26), or `None` for a map-table or end-of-line sub-block. The raw `data` stays alongside it, so the round trip is unchanged.

## Dependencies

`broadcast-common` `9.3` -> `9.4` (see `broadcast-common-9.4.0.md`). The `mpeg-pes` `0.4` -> `0.5` move is in `[dev-dependencies]` only (it reassembles subtitle PES from a TS capture in the integration test), so it does not affect consumers of the library.

---

Published from tag `dvb-subtitle-v0.5.0`.
