# compliance-probe 0.9.0

_Released 2026-11-15._

- `scte35-splice` 6 -> 7: `SpliceTime::pts_time` is a validated `broadcast_common::clock33::Clock33`, and `SpliceInsert` carries its flags and times in one `SpliceMode` (see that crate's changelog).


### Removed
- Removed the unused `metrics-exporter-prometheus` dev-dependency.

---

Published from tag `compliance-probe-v0.9.0`.
