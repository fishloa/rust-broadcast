# compliance-probe 0.10.0

_Released 2026-11-17._

### Changed
- `feed_ts_packet` exports the per-PID `compliance_probe_pcr_drift_ppm`/`_jitter_ppm` gauges only for PIDs a PMT has declared as its `PCR_PID`; a PCR on any other PID counts `compliance_probe_pcr_undeclared_pid_total` instead of creating a labelled series. The estimate itself is unchanged and still available through `conformance::PcrSample`.

### Fixed
- `Probe::drain_byte_tap` resets its byte re-aligner on `TapItem::Lagged`, so a partial packet buffered before a tap gap is discarded instead of being completed with post-gap bytes (a stitched packet reached the conformance monitor and fabricated TR 101 290 / PCR-drift readings).

---

Published from tag `compliance-probe-v0.10.0`.
