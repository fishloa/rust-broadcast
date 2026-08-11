# Changelog

All notable changes to `compliance-probe` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-08-11

### Changed
- `scte35::judge`'s wrap-aware future-vs-past distance now delegates to
  `broadcast_common::clock33::wrapping_forward_distance` — the shared owner
  of this math (a duplication-audit consolidation with `timed-metadata`,
  `transmux`, `media-doctor`, each of which previously hand-rolled the same
  modular-distance formula). Identical computation, no behaviour change; no
  public API change (internal only).
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.
- Repinned `tests/wasm_analyzer_equivalence.rs`, the module docs, `README.md`
  and `examples/fixture_report.rs` to the
  post-#956 `dvb-conformance` numbers on `fixtures/ts/m6-single.ts`:
  `Continuity_count_error` is now **876** (was 803) at every clock rate, and
  the demo-WASM-analyzer reading is now **911** (was 838). No change in this
  crate — `dvb-conformance` #956 fixed the monitor to require a repeated
  packet be byte-identical (bar PCR re-encoding) to count as a legal
  duplicate per ITU-T H.222.0 §2.4.3.3, so the 73 packets that only repeated
  the previous continuity counter without being byte-identical now correctly
  count as `Continuity_count_error`. The cross-tool delta stays exactly
  `[("Buffer_error", 35)]` — the demo WASM analyzer shares the same fix (both
  crates hold a plain path dependency on `dvb-conformance`, so there is no
  duplicated indicator logic to patch twice), so the divergence remains a
  single, understood cause: that analyzer's PCR-anchored clock degenerating
  on a PCR-less fixture, not a second bug.

### Added

Initial implementation (issue #930, `docs/IDEAS.md` item #4). Not yet
published — no `compliance-probe` version exists on crates.io and no
`compliance-probe-v*` tag exists.

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
  readings (911 under that analyzer's clock, 876 under a realistic arrival
  clock) and proving the entire difference is the clock model, not indicator
  logic: both tools use the identical default `dvb_conformance::Config`, the
  fixture carries no PCR (so the analyzer's PCR-anchored clock degenerates to
  1 ns/packet ≈ 1.5 Tbit/s), and every one of the 35 extra events is
  T-STD `Buffer_error` (3.3). Also asserts `Continuity_count_error` is
  clock-independent at 876 across every rate from frozen to 2 ms/packet, so
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
  See the equivalence test above for a measured 911-vs-876 instance.
