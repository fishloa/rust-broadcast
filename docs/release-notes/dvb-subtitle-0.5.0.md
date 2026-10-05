# dvb-subtitle 0.5.0

_Released 2026-10-05._

### Changed (breaking)
- **#1108 (DS-W1)**: `AnySegment` gained a `Malformed { segment_type, page_id,
  data, err }` variant, distinct from `Unknown`. `PesDataField::parse` now
  puts a recognised segment_type whose typed parse returned `Err` into
  `Malformed` instead of folding it into the same `Unknown` raw-passthrough
  a genuinely unimplemented segment_type gets — a broken typed parser could
  otherwise hide behind a byte-exact round-trip forever.
- **#1108 (DS-W2)**: every segment type's `parse` now validates `sync_byte
  == 0x0F` (`Error::BadSyncByte`), not just `PesDataField`'s segment loop.
- **#1108 (DS-W3)**: removed redundant stored counts that could diverge from
  the data they describe: `ObjectDataPayload::Characters.number_of_codes`
  and `DisparityShiftUpdateSequence.division_period_count` are now derived
  from the corresponding `Vec`'s length on serialize.
- **#1108 (DS-W3)**: `DisparityRegion`'s subregion count is now validated to
  be 1..=4 (`Error::InvalidSubregionCount`) — Table 29's
  `number_of_subregions_minus_1` is 2 bits, so a count outside that range is
  no longer silently wrapped by `as u8 & 0x03`. Each `Subregion`'s
  `subregion_horizontal_position`/`subregion_width` presence must also now
  agree with the region's subregion count (`Error::SubregionPositionMismatch`),
  since the position fields are present for every subregion or none.
- **#1108 (DS-W5)**: `PixelDataSubBlock::runs()` decodes a 2/4/8-bit
  pixel-data code string (Tables 22-26) into typed `PixelRun`s
  (`pixel_code`/`run_length`), rather than leaving the caller to walk the
  raw RLE bytes itself. `data` is kept as-is alongside it (needed for the
  byte-exact round-trip).
- **#1108 (DS-W6)**: `InterlacedPixelsData.stuffing_byte: Option<u8>` is now
  `stuffing: &[u8]` — trailing bytes beyond the first were previously
  dropped instead of preserved. `AlternativeClutSegment` and
  `DisparitySignallingSegment` now reject (`Error::TrailingEntryBytes`/
  `Error::BufferTooShort`) a trailing remainder that isn't a whole entry,
  instead of silently dropping it (integer division / a bare loop `break`).
  `PageCompositionSegment`'s `suffix` field is removed — it could never hold
  anything, since parse already rejected a non-whole-entry region loop
  before it was ever populated.
- Serializers now return an error, instead of silently truncating, when a
  segment body exceeds the generic segment header's 16-bit `segment_length`
  field (#1129).
- `RegionCompositionSegment`'s object-entry serializer now returns
  `Error::FieldOverflow` (`#[from] broadcast_common::len::FieldOverflow`)
  instead of silently masking, when `object_horizontal_position` or
  `object_vertical_position` (both 12-bit fields, EN 300 743 Table 11)
  exceeds `0x0FFF` (#1044/#1129).

### Fixed
- **#1044**: `RegionCompositionSegment`'s object-entry serializer wrote
  `object_vertical_position`'s top nibble into the reserved high nibble of
  byte 4 instead of the low nibble (EN 300 743 Table 11: `reserved(4)` then
  the 12-bit position), silently corrupting the position for any object at
  `object_vertical_position >= 256`.
- Every segment type's `serialize_into` (`object_data`, `region_composition`,
  `page_composition`, `clut_definition`, `alternative_clut`,
  `disparity_signalling`, `display_definition`, `stuffing`, and the `Unknown`
  fallback in `any`) no longer silently wraps `segment_length` to a smaller
  value for a body of 64 KiB or more; each now returns `Error::SegmentTooLarge`
  instead (#1108, #1129).

---

Published from tag `dvb-subtitle-v0.5.0`.
