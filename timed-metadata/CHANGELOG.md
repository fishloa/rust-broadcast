# Changelog — timed-metadata

All notable changes to this crate. Format: [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

## [0.1.2] — 2026-07-01
### Added
- emsg version 0 ↔ version 1 conversion (`emsg_to_v0` / `emsg_to_v1` + `SegmentTiming`),
  recomputing the timing field against the segment EPT (`T = EPT + delta` ==
  `presentation_time`), honouring `timescale` equality and carrying PTO for
  Movie↔Period alignment (ISO/IEC 23009-1:2022 §5.10.3.3). Byte-identical
  round-trip verified against real v0 + v1 (DASH-IF livesim) SCTE-35 emsg fixtures.

## 0.1.1 — 2026-06-29

### Changed
- Dependency `broadcast-common` bump (renamed from `dvb-common`); no API change.

## 0.1.0 — 2026-06-27

Initial release.

### Added

- **`TimedEvent`** — canonical hub type carrying the event's abstract kind
  (`EventKind`: `BreakStart`, `BreakEnd`, `Chapter`, `Unspecified`), optional
  media time, duration, and the lossless verbatim source payload (`SourcePayload::Scte35`
  / `SourcePayload::Emsg`).
- **`TimeAnchor`** — maps a 90 kHz PTS to a UTC wall-clock instant; `rfc3339()`
  converts any `MediaTime` to an ISO-8601 string.
- **`DateRange`** — typed `EXT-X-DATERANGE` model with `to_tag_line()` /
  `parse_tag_line()` (RFC 8216 / draft-pantos-hls-rfc8216bis §4.4.5.1).
- **`convert::scte35_to_daterange`** — pure SCTE-35 → DATERANGE edge.
- **`convert::scte35_to_emsg`** / **`convert::emsg_to_scte35`** — pure
  SCTE-35 ↔ DASH `emsg` edges (SCTE 214-3, scheme `urn:scte:scte35:2013:bin`).
- **`Timeline`** — stateful session: holds the `TimeAnchor`, unrolls 33-bit PTS
  wrap, and exposes `push_scte35` / `to_daterange` / `to_emsg`.
- `no_std` + `alloc`; features: `std` (default), `serde` (default), `chrono` (default).
- `label_coverage` drift-guard (CI gate for `EventKind` / `Scte35Cue` labels).

### Deferred (v0.2+)

- SCTE-104 ingest.
- ID3 timed metadata carriage.
- `segmentation_type_id`-based `EventKind` refinement beyond binary out/in.
- `chrono::DateTime` interop helpers behind the `chrono` feature.
