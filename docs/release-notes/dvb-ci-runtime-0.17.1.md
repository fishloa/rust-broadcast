# dvb-ci-runtime 0.17.1

_Released 2026-10-11._

### Changed
- Linux backend: the two `ioctl` calls (`CA_RESET`, `CA_GET_SLOT_INFO`) are isolated in one private `ioctl` module (the only `unsafe` in the crate) behind safe fns; tests use `rustix` (`mkfifoat`, `poll`) instead of `libc`.

---

Published from tag `dvb-ci-runtime-v0.17.1`.
