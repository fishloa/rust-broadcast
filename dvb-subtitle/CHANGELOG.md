# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
