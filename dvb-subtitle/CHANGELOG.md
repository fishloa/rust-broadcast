# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
