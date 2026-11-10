# rtmp-runtime 0.9.1

_Released 2026-11-10._

Fix release (0.9.0 -> 0.9.1): three fixes; no breaking change.

Version: 0.9.0 -> 0.9.1.

## Fixed

- Data-AMF0 `@setDataFrame` forwarded as FLV script tag `onMetaData`.
- `ServerSession` refuses second `connect` while publishing with `RtmpError::UnexpectedState`.
- `BasicHeader`/`MessageHeader` `serialize_into` returns `RtmpError::OutputBufferTooSmall` for too-small buffer.

---

Published from tag `rtmp-runtime-v0.9.1`.
