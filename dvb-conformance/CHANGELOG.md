# Changelog

## [Unreleased]

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
- CRC checking generalised across all well-known SI/PSI PIDs (PAT, CAT, NIT,
  SDT/BAT, EIT, TDT/TOT) plus dynamically discovered PMT PIDs.
- Configurable PCR repetition, PCR discontinuity, and PTS repetition limits
  via new `Config` fields.
- Caller-supplied-time model: `ConformanceMonitor::feed(packet, t)` takes a
  monotonic `Duration` timestamp per packet for all timeout checks.
- Configurable hysteresis and timeout parameters via `Config`.
