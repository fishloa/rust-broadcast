# rtmp-runtime 0.7.2

_Released 2026-10-13._

### Added

- `ProtocolControl::try_to_message` (fallible `to_message`).

### Fixed

- `ClientSession::send_metadata` and the server's connect reply (configured `chunk_size` of 0) return `Err` instead of panicking on an unserializable value.

---

Published from tag `rtmp-runtime-v0.7.2`.
