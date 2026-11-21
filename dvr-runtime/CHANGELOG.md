# Changelog

All notable changes to this crate are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this crate uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- The release requires `broadcast-hls` 0.8.2 (typed `DateRange`/`CustomTag`,
  segment-scoped parse options and incremental renderers) and `dvb-si` 15.2.0
  (the EIT present/following tracker). The manifest floors stay at the
  currently published 0.8.1 / 15.1.0 until the release wave bumps them.

## [0.1.0]

### Added

- The archive format: one HLS Media Playlist per period
  (`pN.m3u8`), one media file per period (`pN.m4s` / `pN.ts`), the
  `EXT-X-RUSTBROADCAST-ARCHIVE` playlist marker and the per-segment
  `EXT-X-RUSTBROADCAST-TIMING` tag. Segments are `EXT-X-BYTERANGE` into the
  period's single media file; one fMP4 init per period, at the file's head.
- `ArchiveWriter`: a sans-IO roll/retention state machine returning an ordered
  `Plan` of `StoreOp`s, committing state only on `complete(PlanOutcome::Done)`.
  Rolls on first segment, sequence gap, target-duration overflow, byte budget,
  playlist size, init change, period duration and programme transition.
- `read::parse_period_playlist` with the torn-line rule, typed validation and
  programme decoding; `read::merge`/`read::window`/`read::checked_range` for
  catch-up planning.
- `recover::scan_fmp4_data`: init and `moof`+`mdat` span recovery from an fMP4
  period file.
- Feature `fs`: `FsRecorder` (blocking plan execution, retry on a partial
  write) and `FsArchive` (scan/find/read/recover plus a bounded per-archive
  parsed-playlist cache).
