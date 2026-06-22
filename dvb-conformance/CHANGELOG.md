# Changelog

## [Unreleased]

## [7.9.0] — 2026-06-22

### Changed

- Lockstep release; no functional changes to this crate.

## [7.8.0] — 2026-06-21

### Changed

- Lockstep release; no functional changes to this crate.

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

### Added
- `examples/`: `monitor_stream` (run the TR 101 290 monitor over a capture) and
  `priority_breakdown` (tally findings by measurement priority + indicator).

## [7.4.0] — 2026-06-18

Lockstep release; no functional changes.

## [7.3.0] — 2026-06-17

### Changed
- Lockstep release; no functional changes to this crate.

## [7.2.0] — 2026-06-16

### Changed
- Lockstep release; no functional changes to this crate.

## [7.1.0] — 2026-06-15

### Changed
- Lockstep release; rebuilt against the dvb-* parser-hardening pass (#207). No
  functional changes.

## [7.0.0] — 2026-06-14

**BREAKING (MSRV 1.75 → 1.81).**

### Added
- **no_std + alloc support** (#63; HashMap→BTreeMap).

### Changed (breaking)
- MSRV **1.81**.

## [6.7.0] — 2026-06-14

### Added
- `Display` on `Priority` and `Indicator`, and `name()` on `Priority`, via
  `impl_spec_display!`; `label_coverage` drift-guard test (#204).

## [6.6.0] — 2026-06-14

Version-lockstep release with the workspace (dvb-t2mi L1-pre/L1-post signalling parser #54; dvb-si BIOP object-carousel layer #64; criterion benchmark suites #62). No changes to this crate.

## [6.5.0] — 2026-06-13

Version-lockstep release with the workspace (#47 T2 emission-time accessors; #50 SSU GroupInfoIndication + data_broadcast_id 0x000A selector; #53 S2Xv2 0x24 extension descriptor). No changes to this crate.

## [6.4.0] — 2026-06-13

Version-lockstep release with the workspace (#158 spec-table drift-guards + spec-fidelity audit; dvb-si PMT section/last-section fields; dvb-bbframe DVB-S2 BUFSTAT ISSY decode). No changes to this crate.

## [6.3.0] — 2026-06-13

Version-lockstep release with the workspace (new `dvb-scte35` crate; dvb-si `TsResync` byte-stream resync helper). No changes to this crate.

## [6.2.0] — 2026-06-13

### Added
- New crate `dvb-conformance`: ETSI TR 101 290 v1.4.1 transport-stream
  conformance monitor (#57).
- Priority-1 indicator set implemented: `TS_sync_loss` (1.1),
  `Sync_byte_error` (1.2), `PAT_error_2` (1.3.a),
  `Continuity_count_error` (1.4), `PMT_error_2` (1.5.a),
  `PID_error` (1.6).
- Priority-2 indicator set implemented: `Transport_error` (2.1),
  `CRC_error` (2.2), `PCR_repetition_error` (2.3a),
  `PCR_discontinuity_indicator_error` (2.3b), `PTS_error` (2.5),
  `CAT_error` (2.6).
- Indicator 2.4 (`PCR_accuracy_error`) is intentionally not implemented: the
  ±500 ns spec tolerance requires hardware arrival timestamps, which are not
  available under the caller-supplied-time model.
- Priority-3 indicator `SI_repetition_error` (3.2, maximum-interval dimension)
  implemented for NIT_actual (10 s), SDT_actual (2 s), EIT P/F actual (2 s),
  and TDT (30 s). Timers are lazily armed — checking starts only after the
  first section of each table is seen.
- The 25 ms minimum-gap dimension of indicator 3.2 is deferred: it needs
  per-`(table_id, section_number)` tracking to avoid false positives on dense
  multi-section tables.
- CRC checking generalised across all well-known SI/PSI PIDs (PAT, CAT, NIT,
  SDT/BAT, EIT, TDT/TOT) plus dynamically discovered PMT PIDs.
- Configurable PCR repetition, PCR discontinuity, and PTS repetition limits
  via new `Config` fields.
- Configurable SI repetition intervals (`si_nit_interval`, `si_sdt_interval`,
  `si_eit_pf_interval`, `si_tdt_interval`) via new `Config` fields.
- Caller-supplied-time model: `ConformanceMonitor::feed(packet, t)` takes a
  monotonic `Duration` timestamp per packet for all timeout checks.
- Configurable hysteresis and timeout parameters via `Config`.
