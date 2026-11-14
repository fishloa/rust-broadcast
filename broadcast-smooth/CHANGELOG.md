# Changelog

All notable changes to `broadcast-smooth` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- `SmoothParseError` gains `From<broadcast_common::xml_chars::XmlCharError>`, the typed character-data and namespace failure the shared XML helper now reports (#2646).
- BREAKING: `SmoothManifest::timescale` is `Timescale` and `StreamIndex::timescale` is `Option<Timescale>` (was `u64` / `Option<u64>`); `StreamIndex::with_timescale` takes a `Timescale`. A present `TimeScale` that is zero, wider than 32 bits or not a number is `SmoothParseError::InvalidAttributeValue`; an absent root `TimeScale` resolves to the new `SMOOTH_DEFAULT_TIMESCALE` (10 000 000) and an absent `StreamIndex@TimeScale` stays `None` (#2657).

### Added
- `SMOOTH_DEFAULT_TIMESCALE`: `SMOOTH_TIMESCALE` as a `Timescale`.

### Removed
- Removed the unused `thiserror` dependency and the `thiserror/std` entry in the `std` feature.

## [0.1.0] - 2026-11-14
Initial release: Microsoft Smooth Streaming (MS-SSTR) client Manifest syntax, parse and write,
moved out of `transmux` so the manifest has one model shared by the packager, the client and the
origin (#2356). Plain `std` (the shared XML reader is std-only); `manifest` is a named feature, on with `std`.

### Added
- The MS-SSTR client Manifest model, `SmoothManifest`/`StreamIndex`/`QualityLevel`/`C`, with `parse`/`parse_with_limits` (through the bounded `broadcast_common::xml` reader) and `to_xml`. It covers the live/DVR attributes, `Protection`/`ProtectionHeader`, `StreamIndex@QualityLevels` and the `c@n` fragment number. `to_xml` writes exactly the model (one `c` per `chunks_list` entry, each attribute only when set), so `parse(to_xml(m)) == m`.
- `SmoothManifest::new`/`with_duration`/`live`/`with_protection` and the `StreamIndex`/`QualityLevel`/`C` builders; the public structs are `#[non_exhaustive]`.
- `StreamIndex::enumerate_chunks`: bounded `c@r` expansion with MS-SSTR §2.2.2.6 semantics: `r` is one-based (`r = 0` is `SmoothParseError::InvalidRepeatCount`), a `c` without `@d` takes its duration from the next `@t` (`MissingDuration` when there is none; `IndivisibleRun` when a run does not divide the span), and an explicit `@t` that repeats or steps back is `NonMonotonicTime`.
- `resolve_fragment_url` (fragment-URL token substitution) and the input caps `MAX_CHUNK_RUN` and `MAX_CODEC_PRIVATE_DATA_HEX_LEN`.
