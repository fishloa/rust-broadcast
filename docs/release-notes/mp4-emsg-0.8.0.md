# mp4-emsg 0.8.0

_Released 2026-11-15._

### Changed
- BREAKING: `EmsgBox::timescale` is a `Timescale` (was `u32`). A zero `timescale` on the wire is the new `Error::InvalidValue` instead of a box that carries an unusable time base. The `serde` feature now also enables `broadcast-common/serde`. Migration: `timescale: 90_000` -> `timescale: Timescale::from_nonzero(90_000)` (#2657).

---

Published from tag `mp4-emsg-v0.8.0`.
