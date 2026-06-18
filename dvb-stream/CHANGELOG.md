# Changelog

## [Unreleased]

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
