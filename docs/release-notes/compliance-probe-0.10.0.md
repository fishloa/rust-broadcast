# compliance-probe 0.10.0

_Released 2026-11-17._

Minor-class release (0.9 -> 0.10) in the audit 20 resource-bounds bundle. It stops exporting PCR drift/jitter series for PIDs a PMT has not declared as `PCR_PID`, and it resets the byte re-aligner on a tap gap so a stitched packet no longer reaches the conformance monitor. The PCR estimate itself is unchanged and remains available through `conformance::PcrSample`.

## Upgrading

- The per-PID `compliance_probe_pcr_drift_ppm`/`_jitter_ppm` gauges are exported only for PIDs a PMT declares as `PCR_PID`; a PCR on any other PID increments `compliance_probe_pcr_undeclared_pid_total`. Update dashboards or alerts that read a series for an undeclared PID.
- `media-doctor` 0.16 -> 0.17 (the requirement is now `0.17.0`).
- `media-plane` 0.12 -> 0.13 (the requirement is now `0.13.0`).

## Changed

- `feed_ts_packet` exports the per-PID `compliance_probe_pcr_drift_ppm`/`_jitter_ppm` gauges only for PIDs a PMT has declared as its `PCR_PID`; a PCR on any other PID counts `compliance_probe_pcr_undeclared_pid_total` instead of creating a labelled series. The estimate itself is unchanged and still available through `conformance::PcrSample`.

## Fixed

- `Probe::drain_byte_tap` resets its byte re-aligner on `TapItem::Lagged`, so a partial packet buffered before a tap gap is discarded instead of being completed with post-gap bytes (a stitched packet reached the conformance monitor and fabricated TR 101 290 / PCR-drift readings).

---

Published from tag `compliance-probe-v0.10.0`.
