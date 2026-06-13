# Changelog

## [Unreleased]

## [6.3.0] — 2026-06-13

### Changed
- `for_each_packet` now drives the shared `dvb_si::ts::TsResync` (188/204-byte
  resync helper) instead of ad-hoc 188-byte chunking (#61); behaviour for
  aligned input is unchanged.

## [6.2.0] — 2026-06-13

### Added
- **`dvb-tools`** (#59) — new published binary crate over the `rust-dvb`
  family. Five subcommands, all driven over aligned 188-byte `.ts` captures
  (no new dependencies — argument parsing stays on `std::env::args`):
  - `dump [--json]` — SI section dump (`SiDemux`-driven, ported from the old
    `si_dump` example).
  - `services` — SDT + NIT service tree with LCNs
    (`SectionSetCollector` + `CompleteSdt`/`CompleteNit`).
  - `epg [--json]` — EPG schedule via `EpgStore`; EIT events with service
    names attached from the SDT (`feed_sdt`).
  - `pids` — per-PID packet counts, sorted by descending packet count,
    with bitrate estimated from the first/last PCR observed.
  - `t2mi [--pid 0xNNN|raw] [--inner] [--plp N]` — T2-MI pump
    (ported from the old `t2mi_dump` example); with `--inner`, chain-unwrap
    to the inner MPEG-TS via `InnerTsRecovery` and write the recovered
    188-byte packets to stdout. `--plp` targets one baseband frame's PLP.
