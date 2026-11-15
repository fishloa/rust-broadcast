# atsc3-route 0.6.0

_Released 2026-11-15._

Minor-class release (0.5 -> 0.6). `rmt-flute` now decodes a ROUTE packet's transport session and object identifiers into typed `Tsi`/`Toi` values instead of handing back borrowed bytes, and `atsc3-route` hands those same values out, so a caller builds and reads a ROUTE identifier through the type rather than by converting a byte slice.

Version: 0.5.0 -> 0.6.0.

## Breaking changes and migration

A `RoutePacket`'s LCT identifiers are `rmt_flute::{Tsi, Toi}` (through `RoutePacket::lct`), built with `Tsi::new`/`Toi::new` rather than raw `to_be_bytes()` slices. A/331 fixes both widths (S = 1, H = 0 gives a 4-byte TSI; O = 01, H = 0 a 4-byte TOI), and `Tsi::new`/`Toi::new` reject a width the flags cannot describe and a value the width cannot hold.

```rust
// before: the identifiers were the raw wire bytes
let tsi_bytes: &[u8] = &packet.lct.tsi;
let toi_bytes: &[u8] = &packet.lct.toi;
// after
let tsi: rmt_flute::Tsi = packet.lct.tsi;
let toi: rmt_flute::Toi = packet.lct.toi;
let (tsi_value, toi_value) = (tsi.value(), toi.value());
let built = rmt_flute::Tsi::new(3000, 4)?;   // ROUTE's 4-byte TSI
```

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).
- `rmt-flute` 0.9 -> 0.10.

---

Published from tag `atsc3-route-v0.6.0`.
