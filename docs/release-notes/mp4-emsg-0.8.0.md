# mp4-emsg 0.8.0

_Released 2026-11-15._

Minor-class release (0.7 -> 0.8). An `emsg` box's timescale is the shared `Timescale`, so a box can no longer carry an unusable time base: a zero `timescale` on the wire is a typed error at parse instead of a value every consumer would have to guess the meaning of.

Version: 0.7.0 -> 0.8.0.

## Breaking changes and migration

### `EmsgBox::timescale` is a `Timescale`

`EmsgBox::timescale` is a `Timescale` (was `u32`). A zero `timescale` on the wire is the new `Error::InvalidValue` instead of a box that carries an unusable time base. The `serde` feature also enables `broadcast-common/serde`, so the timescale serializes.

```rust
// before
let b = EmsgBox { timescale: 90_000, .. };
let hz: u32 = b.timescale;
// after
let b = EmsgBox { timescale: Timescale::try_new(90_000)?, .. };
let hz: u32 = b.timescale.get();
```

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).

---

Published from tag `mp4-emsg-v0.8.0`.
