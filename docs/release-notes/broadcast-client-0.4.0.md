# broadcast-client 0.4.0

_Released 2026-11-08._

Breaking-class release (0.3 -> 0.4) only because `rtsp-runtime` moves to 0.10, which changes the caret epoch this crate builds against. This crate's own public API is unchanged.

Version: 0.3.0 -> 0.4.0.

## Dependencies

Workspace sibling requirements move to the new epochs of this release:

- `rtsp-runtime` 0.9 -> 0.10

`rtsp-runtime::Error` now carries its source errors and no longer derives `Clone`, `PartialEq` or `Eq` (#1726, see the rtsp-runtime 0.10.0 note). `broadcast-client`'s own `Error` does not expose it, so there is no migration here. A project that names `broadcast-client` and `rtsp-runtime` together must move both.

---

Published from tag `broadcast-client-v0.4.0`.
