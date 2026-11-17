# dvb-conformance 15.1.0

_Released 2026-11-17._

Minor release (15.0 -> 15.1) adding `ConformanceMonitor::declares_pcr_pid`, which reports whether a PID is the `PCR_PID` the latest PMT of a PAT-referenced program declares. `compliance-probe` uses it to restrict its per-PCR-PID series to the PIDs the stream designates. No existing API changes.

## Added

- `ConformanceMonitor::declares_pcr_pid`: whether a PID is the `PCR_PID` the latest PMT of a PAT-referenced program declares, so a consumer exporting per-PCR-PID series can restrict them to the PIDs the stream designates.

---

Published from tag `dvb-conformance-v15.1.0`.
