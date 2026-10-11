# transmux 0.25.1

_Released 2026-10-11._

### Fixed

- `tkhd`/`mdhd` version-1 parsing reads its 64-bit fields through a checked `be_u64` helper (`BufferTooShort` instead of `try_into().unwrap()`); behaviour identical for valid input.

---

Published from tag `transmux-v0.25.1`.
