# Changelog

All notable changes to `rtmp-runtime` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- `examples/capture_publish.rs` (feature `tokio`): a real-socket RTMP publish
  recorder driving `ServerSession` over a live TCP connection, used to
  capture `tests/fixtures/obs-publish.bin` (a real `ffmpeg -f flv` publish).
- `tests/ingest_fixture.rs`: replays the captured fixture through
  `ServerSession::handle_data` (single-call and small-chunk reassembly) and
  feeds the emitted FLV to `transmux::FlvDemux`, asserting a decoded
  H.264+AAC `Media` — the real-fixture end-to-end ingest test.

### Fixed
- `ServerSession::handle_data` now dispatches each reassembled message as
  soon as it is parsed, instead of collecting a whole `ChunkAssembler::push`
  batch before dispatching any of them. A client Set Chunk Size (§5.4.1)
  arriving in the same `handle_data` call as chunks already framed at the
  new size — exactly what a real `ffmpeg` publisher does — was previously
  misparsed, since the old chunk size stayed in effect for the rest of that
  batch. Caught by the real `ffmpeg` capture fixture above.
- `ChunkAssembler` gained crate-internal `feed`/`next_message` (incremental,
  one-message-at-a-time parsing) alongside the existing `push`, which
  `ServerSession` now uses for this reason.
