# Changelog

## [0.4.0] - 2026-07-29

### Changed (BREAKING)
- **Requires `dvb-si` 9 and `dvb-t2mi` 9** (issue #819). No functional change.

  The published 0.3.1 still required `^8` of both as *normal* dependencies, so
  a consumer combining it with `dvb-si` 9 got two majors of the same crate in
  one graph and the `Parse`/`Serialize` impls belonged to the wrong one. This
  was missed by the #819 sweep, which only checked `broadcast-common`
  requirements -- `dvb-stream`'s broadcast-common dependency is dev-only, so it
  did not show up. Found by the new published-dependency consistency check
  (#821) on its first run, which is the argument for having it.

## [0.4.1] - 2026-07-30

### Fixed
- Floor `mpeg-ts` to `0.3.1`. The `^0.3` bucket also contains 0.3.0, which is
  built against `broadcast-common` 8, so a consumer could resolve two
  `broadcast-common` majors into one graph and hit trait-resolution errors
  pointing at this crate's internals (#858).

## [0.4.0] - 2026-07-29

### Changed (BREAKING)
- **Requires `dvb-si` 9 and `dvb-t2mi` 9** (issue #819). No functional change.

  The published 0.3.1 still required `^8` of both as *normal* dependencies, so
  a consumer combining it with `dvb-si` 9 got two majors of the same crate in
  one graph and the `Parse`/`Serialize` impls belonged to the wrong one. This
  was missed by the #819 sweep, which only checked `broadcast-common`
  requirements -- `dvb-stream`'s broadcast-common dependency is dev-only, so it
  did not show up. Found by the new published-dependency consistency check
  (#821) on its first run, which is the argument for having it.

## [Unreleased]

### Added
- A crate-root note recording this crate's exemption from both #806
  drift guards (it defines no `pub enum`). No functional change.

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
