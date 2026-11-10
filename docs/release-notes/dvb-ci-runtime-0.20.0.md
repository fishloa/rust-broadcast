# dvb-ci-runtime 0.20.0

_Released 2026-11-10._

Breaking-class release (0.19 -> 0.20): `resource::Mmi` becomes a struct with chain state; sibling epochs move.

Version: 0.19.0 -> 0.20.0.

## Breaking changes and migration

- `resource::Mmi` now holds chain state, so the unit-struct literal `Mmi` no longer compiles; use `Mmi::default()`.

```rust
// before
let m = Mmi;
// after
let m = Mmi::default();
```

- Added `CaError::NotSent` (`#[non_exhaustive]`).

## Fixed

- `Driver::add_service`/`update_service` return `CaError::NotSent` when no ca_pmt reached CAM.
- Transport re-sends `Create_T_C` up to `MAX_SETUP_ATTEMPTS` (5).
- Reply timeout discards partial `T_Data_More` chain.
- Closing unallocated session answered `SessionStatus::ResourceNonExistent`.
- `Driver::pump` advances stack timers on frames.
- MMI reassembles chained menus/lists.

## Dependencies

- `dvb-si` 12 -> 13
- `broadcast-common` 10 -> 11

---

Published from tag `dvb-ci-runtime-v0.20.0`.
