# Changelog

## [Unreleased]

### Changed
- CLI rebuilt on `clap` (derive) per the workspace CLI standard
  (`docs/CLI-STANDARD.md`): auto-generated `--help`/`--version`, per-subcommand
  `--help`, and proper argument validation. Subcommands take a positional
  `<FILE>` plus named flags (`--json`, `--pid`, `--inner`, `--plp`).

## [7.9.0] — 2026-06-22

### Changed

- Lockstep release; no functional changes to this crate.

## [7.8.0] — 2026-06-21

### Added
- `dump` human mode now decodes and prints the descriptor loops of PMT
  (program-info + per-ES), SDT (per-service), and NIT (network + per-TS).
  Descriptors resolve through a registry with the PDS-scoped 0x83
  logical_channel enabled for the EACEM/NorDig `private_data_specifier`s, so
  LCNs show as `LOGICAL_CHANNEL` instead of `UNKNOWN(0x83)`.

## [7.7.1] — 2026-06-21

### Changed
- Lockstep release; no functional changes to this crate.

## [7.7.0] — 2026-06-20

### Changed
- Lockstep release; no functional changes to this crate.

## [7.6.0] — 2026-06-20

### Changed
- Lockstep release; no functional changes to this crate.

## [7.5.0] — 2026-06-19

### Changed
- Lockstep release; no functional changes to this crate.

## [7.4.0] — 2026-06-18

Lockstep release; no functional changes.

## [7.3.0] — 2026-06-17

### Changed
- Lockstep release; no functional changes to this crate.

## [7.2.0] — 2026-06-16

### Fixed
- `services` now resolves logical channel numbers: the NIT walk uses a
  `DescriptorRegistry` with PDS-scoped logical_channel enabled for EACEM/NorDig,
  so 0x83 decodes as `LogicalChannel` instead of `Unknown` (#211).

## [7.1.0] — 2026-06-15

### Changed
- Lockstep release; rebuilt against the dvb-* parser-hardening pass (#207). No
  functional changes.

## [7.0.0] — 2026-06-14

**BREAKING (MSRV 1.75 → 1.81).**

### Changed
- MSRV **1.81**; tracks the breaking library changes (BIOP typing, typed coded
  fields, `#[non_exhaustive]` enums). Stays a std-only application.

## [6.7.0] — 2026-06-14

### Changed
- Lockstep release with the library crates; no functional changes (consumes the
  #204 `name()`/`Display` label convention).

## [6.6.0] — 2026-06-14

Version-lockstep release with the workspace (dvb-t2mi L1-pre/L1-post signalling parser #54; dvb-si BIOP object-carousel layer #64; criterion benchmark suites #62). No changes to this crate.

## [6.5.0] — 2026-06-13

Version-lockstep release with the workspace (#47 T2 emission-time accessors; #50 SSU GroupInfoIndication + data_broadcast_id 0x000A selector; #53 S2Xv2 0x24 extension descriptor). No changes to this crate.

## [6.4.0] — 2026-06-13

Version-lockstep release with the workspace (#158 spec-table drift-guards + spec-fidelity audit; dvb-si PMT section/last-section fields; dvb-bbframe DVB-S2 BUFSTAT ISSY decode). No changes to this crate.

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
