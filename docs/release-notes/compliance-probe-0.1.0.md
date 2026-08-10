# compliance-probe 0.1.0

**Release date:** 2026-08-10

Initial release (issue #930, `docs/IDEAS.md` item #4). `compliance-probe` is a live probe over a `media-plane` `Trunk`/`ByteTap`: it drives `dvb-conformance`'s ETSI TR 101 290 indicators, a PCR-drift/jitter estimate, and SCTE-35 `splice_insert` cue-sanity checks, exporting everything through the `metrics` facade for a host process (e.g. `multimux`) to render as Prometheus. **It does not open a socket, run a background thread, or serve `/metrics` itself** — it is a library the host feeds and reads back through the metrics recorder the host already installed. `std` only. Not yet published — no `compliance-probe` version exists on crates.io and no `compliance-probe-v*` tag exists yet.

## What's in it

- `Probe::feed_ts_packet` — drives `dvb_conformance::ConformanceMonitor` over a live TS packet stream, recording every TR 101 290 indicator event through the `metrics` facade (`compliance_probe_tr101290_events_total`, `compliance_probe_tr101290_in_sync`).
- `conformance::PcrTracker` (internal) / `compliance_probe_pcr_drift_ppm` + `compliance_probe_pcr_jitter_ppm` — a per-PID software-clock estimate of PCR interval-error, explicitly distinct from TR 101 290's own `PCR_accuracy_error` (2.4).
- `scte35::check_section` / `Probe::feed_scte35_section` — SCTE-35 `splice_insert` sanity: well-formedness, cue arrival by kind, and wrap-aware future-vs-past `pts_time` judgement (`Scte35Sanity`/`compliance_probe_scte35_*` metrics). The future-vs-past distance now delegates to `broadcast_common::clock33::wrapping_forward_distance` rather than a local copy of the same modular-distance formula.
- `structural::run_structural_checks` — a thin passthrough to `media-doctor`'s `Diagnostic` harness, recording `compliance_probe_structural_findings_total`.
- `trunk_bridge` (`std` only) — `Probe::drain_byte_tap` over a `media_plane::ByteTap` and `Probe::drain_event_cursor` over a `media_plane::EventCursor`, each a bounded, non-blocking observer rather than a second copy of the stream.
- `dashboards/compliance-probe.json` — a Grafana dashboard covering every metric this crate exports, including a panel documenting what is deliberately not monitored, and a priority-1 TR 101 290 alert rule.
- `tests/wasm_analyzer_equivalence.rs` — cross-tool equivalence against the `demo/` WASM analyzer over `fixtures/ts/m6-single.ts`, pinning both readings (911 under that analyzer's clock, 876 under a realistic arrival clock) and showing the entire difference traces to the clock model, not indicator logic.

## What is deliberately NOT measured

Stated plainly so a green dashboard means "checked and passing", never "not checked":

- **TR 101 290 indicator 2.4, `PCR_accuracy_error`** — `dvb-conformance` itself never emits it (it needs ±500 ns hardware arrival timing a sans-IO monitor cannot honestly provide), and this crate adds no replacement.
- **T-STD buffer-model sub-checks** that `dvb-conformance` documents as partial (`Buffer_error` TBn overflow; `Empty_buffer_error` MBn; `Data_delay_error`'s still-picture 60 s threshold) — this crate adds no buffer-model logic of its own.
- **SCTE-35 malformed-section detection on the Trunk-cursor path** — an event already published into a `media_plane::Trunk`'s event log was, by construction, already parsed successfully upstream; only the wire path (`Probe::feed_scte35_section`) can observe malformedness.
- **A fabricated "now" for an unresolved SCTE-35 anchor** — a `media_plane::EventAnchor::Segment`/`Utc` entry is left exactly that, never guessed into a `Media` time.
- **The caller-supplied arrival clock is an input to the result, not bookkeeping** — `Buffer_error`/`Empty_buffer_error`/`Data_delay_error` are statements about arrival timing and are only as trustworthy as the clock fed to `Probe::feed_ts_packet`.

## Migration

New crate — no migration needed. MSRV is **1.95.0**.
