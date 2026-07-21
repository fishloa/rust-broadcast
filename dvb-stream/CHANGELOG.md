# Changelog

## [0.3.1] - 2026-07-21
### Changed
- Widen the internal `mpeg-ts` dependency to `0.3` (was `0.2`; issue #663;
  private dependency — no public API change to `dvb-stream`).

## [0.3.0] - 2026-07-03
### Changed
- Rust **edition 2024**; MSRV raised to **1.86**; format-argument modernisation. No functional or API change.

## [0.2.2] — 2026-06-29

### Changed
- Dependency `broadcast-common` bump (renamed from `dvb-common`); no API change.

## [0.2.1] — 2026-06-19

### Added
- `examples/`: `count_sections` (drive `SectionStream` over an in-memory TS) and
  `stream_stats` (tally table types + report demux/resync stats).

## [0.2.0] — 2026-06-16

### Added
- `ResyncStats { resyncs, bytes_discarded, desyncs }` + a `resync_stats()`
  accessor on `SectionStream` and `T2miStream`. `feed_buf` now counts re-aligns
  and discarded bytes, and **detects mid-stream desync** (a fed packet not
  starting with the `0x47` sync byte): it increments `desyncs`, discards the rest
  of the chunk, and forces a re-resync on the next read — instead of silently
  slicing garbage on corrupted mid-stream data (#220). Byte-identical for
  well-formed streams (counters stay zero).

### Changed
- Dependency requirements on the core crates bumped to `7.2`.

## [0.1.0]

Initial release — `SectionStream` / `T2miStream` async adapters over
`dvb_si::SiDemux` / `dvb_t2mi::T2miPump` with 188-byte TS resync.
