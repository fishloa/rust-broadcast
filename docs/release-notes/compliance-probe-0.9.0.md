# compliance-probe 0.9.0

_Released 2026-11-15._

Dependency/epoch bump only: the probe's own API is unchanged; it moves so that its published bucket names the sibling releases it is known to build against, including `scte35-splice` 7 whose `SpliceTime::pts_time` is the validated `Clock33` and whose `SpliceInsert` carries its flags and times in one `SpliceMode`.

Version: 0.8.0 -> 0.9.0.

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).
- `dvb-conformance` 14 -> 15, `media-doctor` 0.15 -> 0.16, `media-plane` 0.11 -> 0.12, `mpeg-ts` 0.8 -> 0.9, `scte35-splice` 6 -> 7, `timed-metadata` 0.10 -> 0.11.
- The unused `metrics-exporter-prometheus` dev-dependency is removed.

---

Published from tag `compliance-probe-v0.9.0`.
