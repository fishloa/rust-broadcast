# Changelog

All notable changes to `compliance-probe` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-08-09

Initial release (issue #930, `docs/IDEAS.md` item #4).

### Added
- `Probe::feed_ts_packet` — drives `dvb_conformance::ConformanceMonitor` over
  a live TS packet stream, recording every TR 101 290 indicator event
  through the `metrics` facade (`compliance_probe_tr101290_events_total`,
  `compliance_probe_tr101290_in_sync`).
- `conformance::PcrTracker` (internal) / `compliance_probe_pcr_drift_ppm` +
  `compliance_probe_pcr_jitter_ppm` — a per-PID software-clock estimate of
  PCR interval-error, explicitly distinct from TR 101 290's unimplementable
  `PCR_accuracy_error` (2.4).
- `scte35::check_section` / `Probe::feed_scte35_section` — SCTE-35
  `splice_insert` sanity: well-formedness, cue arrival by kind, and
  wrap-aware future-vs-past `pts_time` judgement
  (`Scte35Sanity`/`compliance_probe_scte35_*` metrics).
- `structural::run_structural_checks` — thin passthrough to `media-doctor`'s
  `Diagnostic` harness, recording `compliance_probe_structural_findings_total`.
- `trunk_bridge` (`std` only) — `Probe::drain_byte_tap` over a
  `media_plane::ByteTap` (the wire-level attachment point TR 101 290 needs)
  and `Probe::drain_event_cursor` over a `media_plane::EventCursor` (the
  Trunk-cursor attachment point for already-demuxed SCTE-35 cues), each
  costing exactly one bounded, non-blocking observer.
- `dashboards/compliance-probe.json` — a Grafana dashboard covering every
  metric this crate exports, including a panel documenting what is
  deliberately not monitored, and a priority-1 TR 101 290 alert rule. The
  PCR rate-estimate row is titled and annotated so it cannot be misread as
  TR 101 290 indicator 2.4 (which is not measured at all).
- `tests/wasm_analyzer_equivalence.rs` — cross-tool equivalence against the
  `demo/` WASM analyzer over `fixtures/ts/m6-single.ts`, pinning **both**
  readings (838 under that analyzer's clock, 803 under a realistic arrival
  clock) and proving the entire difference is the clock model, not indicator
  logic: both tools use the identical default `dvb_conformance::Config`, the
  fixture carries no PCR (so the analyzer's PCR-anchored clock degenerates to
  1 ns/packet ≈ 1.5 Tbit/s), and every one of the 35 extra events is
  T-STD `Buffer_error` (3.3). Also asserts `Continuity_count_error` is
  clock-independent at 803 across every rate from frozen to 2 ms/packet, so
  the two tools stay mutually checkable.

### Documented gaps (not defects — read before filing one)
- `PcrAccuracyError` (TR 101 290 2.4) is never emitted by `dvb-conformance`
  and this crate adds no replacement for it.
- SCTE-35 malformed-section detection is wire-path-only; the Trunk-cursor
  path structurally cannot observe it (the event was already parsed
  upstream before being published).
- `structural::run_structural_checks` is whole-buffer-shaped, not
  incremental — see that module's docs for cadence caveats.
- **The caller-supplied arrival clock is an input to the result**, not
  bookkeeping: `Buffer_error`/`Empty_buffer_error`/`Data_delay_error` are
  statements about arrival timing and are only as trustworthy as that clock.
  See the equivalence test above for a measured 838-vs-803 instance.
