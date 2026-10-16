# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `mpeg-pes` 0.5 -> 0.6). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

### Changed (BREAKING, #1202)
- **BREAKING (behavioural):** serializers reject a value wider than its wire bit-field with `Error::FieldOverflow` instead of shifting/masking it into a different valid-looking segment (the #1129 class): the `*_version_number` nibbles, `reserved*` runs, `page_state` / `region_level_of_compatibility` / `region_depth` / `object_coding_method` codes, the 4/2-bit region pixel codes, the alternative-CLUT parameter fields and 8/10-bit entry values, and the disparity-signalling fractional shift / reserved flags. The `Reserved(v)` payload of those code enums is now returned verbatim by `to_bits` (so the serializer's range check can see a wide value) instead of being masked. Guarded by `tests/serialize_mask_guard.rs` (lexical) and `tests/serialize_field_width.rs` (real-capture segments, one field one step over range).
- New `tests/hostility.rs`: real M6 data fields truncated at every length and mutated at every byte never panic.

### Fixed

- **`disparity_shift_update_sequence` `interval_duration` over 24 bits is rejected (#1129)** (was `as u8` of its three bytes, silently dropping the top byte); returns `Error::FieldOverflow`.

### Changed (BREAKING)
- The CamelCase constant re-exports are now SCREAMING_SNAKE: `DataIdentifier`
  -> `DATA_IDENTIFIER`, `EndOfPesMarker` -> `END_OF_PES_MARKER`,
  `SubtitleStreamId` -> `SUBTITLE_STREAM_ID`, `SyncByte` -> `SYNC_BYTE`
  (#1108 DS-O1).
- `ClutDefinitionSegment` serialize now rejects a reduced-range CLUT entry whose
  Y/Cr/Cb/T value is wider than its 6/4/4/2-bit slot, and a `reserved` flags
  value outside bits `[4:1]`, with the new `Error::InvalidValue`; previously the
  value was shifted into its neighbour's bits (#1108 DS-O2).

## [0.5.0] - 2026-10-05
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
  `PageCompositionSegment`'s internal `pub(crate)` `suffix` field is removed
  (not public API, so not itself a break) — it could never hold
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

## [0.4.0] - 2026-08-11

### Changed
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.
### Added
- `tests/label_coverage.rs` + `tests/non_exhaustive_coverage.rs` drift guards
  (issue #806). No public API or behaviour change.

## [0.3.0] - 2026-07-29

### Changed (BREAKING)
- **Requires `broadcast-common` 9** (issue #819). No functional or API change of
  this crate's own.

  Staying on `broadcast-common` 8 was not neutral: this crate's types implement
  `Parse`/`Serialize` from whichever major it links, so a consumer that used it
  alongside a 9-based crate (`transmux` 0.20, `dvb-si` 9, …) got **both majors
  in one graph**, and the trait methods resolved against the wrong one —
  surfacing as `no method named to_bytes found` / `no function named parse
  found` on types that plainly have them, with the compiler pointing at
  `broadcast-common-8.x/src/traits.rs`.

  The 9.0.0 wave originally shipped only the crates needed to publish
  `transmux`/`media-plane`/`multimux`, on the reasoning that everything else
  stayed coherent on its own 8 line. That reasoning was wrong: these crates
  exist to be composed, and the breakage only appears in a consumer that mixes
  them.

## [0.2.0] - 2026-07-03
### Changed
- Rust **edition 2024**; MSRV raised to **1.86**; format-argument modernisation. No functional or API change.

## [0.1.2] — 2026-06-29

### Changed
- Dependency `broadcast-common` bump (renamed from `dvb-common`); no API change.

## [0.1.1] — 2026-06-27

### Changed
- Depend on `mpeg-pes` (renamed from `dvb-pes`) as dev-dependency; no behaviour change.

## [0.1.0]

### Added

- Initial release: parser and serializer for ETSI EN 300 743 V1.6.1 DVB subtitling segments.
- `PesDataField` top-level structure (data_identifier, subtitle_stream_id, segment loop, end marker).
- All segment types from §7.2: display definition, page composition, region composition,
  CLUT definition, object data (incl. 2/4/8-bit pixel-data sub-blocks, character strings,
  progressive pixel blocks), disparity signalling, alternative CLUT, end of display set,
  and stuffing.
- `AnySegment` dispatch enum with `declare_segments!` macro pattern and drift test.
- `SegmentDef` trait for typed segment dispatch.
- Spec-field enums with `name()` + `impl_spec_display!`: PageState, RegionLevelOfCompatibility,
  RegionDepth, ObjectType, ObjectProviderFlag, ObjectCodingMethod, DataType,
  OutputBitDepth, DynamicRangeColourGamut.
- `Parse<'a>` / `Serialize` implementations with byte-identical round-trip tests.
- `#![no_std]` + `alloc`; optional `serde` feature.
- Two runnable examples (`parse_segment`, `parse_full_pes`).

[Unreleased]: https://github.com/fishloa/rust-dvb/compare/v0.1.0...HEAD
