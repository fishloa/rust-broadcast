# atsc3-route 0.6.0

_Released 2026-11-15._

### Changed
- BREAKING: follows `rmt-flute`'s typed `Tsi`/`Toi` — a `RoutePacket`'s LCT identifiers are built with `Tsi::new`/`Toi::new` rather than raw `to_be_bytes()` slices (#2770).

---

Published from tag `atsc3-route-v0.6.0`.
