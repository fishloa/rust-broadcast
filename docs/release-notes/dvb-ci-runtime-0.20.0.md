# dvb-ci-runtime 0.20.0

_Released 2026-11-14._

Breaking-class release (0.19 -> 0.20): breaking API changes are listed below, and the crate's sibling dependencies move to new caret epochs.

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
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12, `dvb-ci` 0.11 -> 0.12, `dvb-si` 13 -> 14, `mpeg-ts` 0.7 -> 0.8.

---

Published from tag `dvb-ci-runtime-v0.20.0`.
