# rtmp-runtime 0.7.1

_Released 2026-10-11._

### Fixed
- `chunk::ChunkWriter::write` returns the new `RtmpError::InvalidChunkStreamId { chunk_stream_id }` for a `Message::chunk_stream_id` outside `2..=65599` instead of panicking. Not breaking: the signature is unchanged and `RtmpError` is `#[non_exhaustive]`.

---

Published from tag `rtmp-runtime-v0.7.1`.
