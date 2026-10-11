# dvb-csa 0.3.1

_Released 2026-10-11._

### Changed
- Zeroing of control-word-derived state now delegates to the `zeroize` crate (no_std) instead of a hand-rolled `write_volatile` loop; behaviour and public API unchanged.

---

Published from tag `dvb-csa-v0.3.1`.
