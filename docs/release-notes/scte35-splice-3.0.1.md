# scte35-splice 3.0.1

_Released 2026-10-13._

### Changed

- `PTS_MODULUS`/`pts_add_wrapping` and the DVB-TA 33-bit mask now delegate to `broadcast_common::clock33` (no behaviour change for valid input); added `tests/clock33_guard.rs`.

---

Published from tag `scte35-splice-v3.0.1`.
