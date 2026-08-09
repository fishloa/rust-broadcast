# Changelog

All notable changes to `rtmp-runtime` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.

## [0.5.0] - 2026-08-07

### Added
- `client` module — sans-IO RTMP 1.0 client publish session engine
  (`ClientSession`, `ClientHandshake`, `ClientConfig`, `ClientEvent`):
  `connect` → `createStream` → `publish` auto-advance,
  `send_audio()`/`send_video()`/`send_metadata()` for the Publishing state
  (issue #744).

## [0.4.0] - 2026-08-05

### Changed
- Requires `transmux` 0.23 (epoch-pure caret bump from ^0.21; `TrackSpec`
  is part of this crate's public ingest API).

## [0.3.0] - 2026-07-30

### Changed (Breaking)
- `LimitType`, `Fmt`, `MessageHeader` (`chunk`, `message`) now carry
  `#[non_exhaustive]` (issue #806's non_exhaustive drift-guard audit). A
  downstream `match` on any of these now needs a wildcard arm.

### Added
- `tests/non_exhaustive_coverage.rs` drift guard (issue #806).

## [0.2.0] - 2026-07-29

### Changed (BREAKING)
- **Requires `broadcast-common` 9.** No functional or API change of this
  crate's own; the bump exists solely to carry the new requirement.
  `broadcast-common` 9.0.0 changed `Encrypt::encrypt` to take `&mut self` (so a
  stateful implementor can own a running per-key IV counter — it fixes a
  duplicate-IV/two-time-pad defect), and its `Parse`/`Serialize` traits appear
  in this crate's public API, so a consumer cannot mix a `broadcast-common` 8
  build with this one. That makes it a breaking release even though no line of
  logic here moved.

## [0.1.0] - 2026-07-26

### Added
- `examples/capture_publish.rs` (feature `tokio`): a real-socket RTMP publish
  recorder driving `ServerSession` over a live TCP connection, used to
  capture `tests/fixtures/obs-publish.bin` (a real `ffmpeg -f flv` publish).
- `tests/ingest_fixture.rs`: replays the captured fixture through
  `ServerSession::handle_data` (single-call and small-chunk reassembly) and
  feeds the emitted FLV to `transmux::FlvDemux`, asserting a decoded
  H.264+AAC `Media` — the real-fixture end-to-end ingest test.
- `serde` feature now actually derives `Serialize`/`Deserialize` on the owned
  public wire/event types (`ServerEvent`, `ServerConfig`, `Amf0Value`,
  `Command`), with a round-trip test gated on the feature.
- `ServerConfig::with_expected_stream_key`/`with_chunk_size`/
  `with_window_ack_size`/`with_peer_bandwidth` builder methods, needed now
  that `ServerConfig` is `#[non_exhaustive]`.

### Fixed
- **Remote excessive-allocation DoS**: `ChunkAssembler` no longer
  `Vec::with_capacity`s an inbound `message_length` (a fully
  attacker-controlled 24-bit wire field, up to ~16 MiB) before a single
  payload byte has arrived for it — the buffer instead grows incrementally
  as real chunk payload shows up. Added `MAX_MESSAGE_LEN` (8 MiB): a
  Type 0/1 header declaring a larger `message_length` is rejected before
  any buffer is allocated. Added `MAX_CSIDS` (64): a flood of chunks opening
  more than this many distinct, previously-unseen chunk stream ids is
  rejected rather than growing the per-csid state map without bound.
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
- `ChunkAssembler`/`ChunkWriter::set_chunk_size` now also cap the negotiated
  chunk size at `MAX_CHUNK_SIZE` (16 MiB), in addition to the existing
  floor-of-1.
- `Amf0Value`, `ProtocolControl`, and `UserControl` are now `#[non_exhaustive]`
  (each models a documented subset of its spec catalogue).
- `read_utf8_short`/`read_utf8_long` (AMF0 String/Long String length
  prefixes) now use `checked_add` instead of a bare `+` when computing the
  total consumed length, guarding against `usize` overflow on 32-bit
  targets.
- `ServerSession`'s `next_stream_id` counter now uses `saturating_add`
  instead of a bare `+= 1`.
