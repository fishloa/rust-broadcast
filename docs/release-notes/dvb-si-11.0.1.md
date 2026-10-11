# dvb-si 11.0.1

_Released 2026-10-11._

### Fixed

- Production parsers no longer use `first_chunk::<N>().unwrap()` / `try_into().unwrap()` for big-endian field reads; they go through a checked `wire::be_u16`/`be_u32` helper that returns `Error::BufferTooShort` instead of being able to panic. Behaviour is byte-identical for valid input; public API unchanged.

---

Published from tag `dvb-si-v11.0.1`.
