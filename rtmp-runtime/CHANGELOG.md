# Changelog

All notable changes to `rtmp-runtime` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- `AsyncRtmpClient` gains sans-IO `encode_video`/`encode_audio`/`encode_metadata`
  (returning the chunk-stream-framed message bytes without writing) and
  `write_frame` (writing one already-framed message under `RtmpTimeouts::write`),
  so a caller driving its own egress can frame and write separately.
  `send_video`/`send_audio`/`send_metadata` now delegate to the `encode_*` half.

### Changed (breaking)
- `RtmpConnection` is now `RtmpConnection<S = TcpStream>` (a `tokio_util::codec::Framed` over the sans-IO session) with `RtmpConnection::from_stream(stream, session, RtmpTimeouts)`; `AsyncRtmpServer` gains `with_timeouts`. New `io::RtmpTimeouts` (connect / handshake / read_idle / write; defaults 10 s / 10 s / 30 s / 10 s): a deadline expiry from `next_events` is `io::ErrorKind::TimedOut` and closes the connection. The `pending_write` field is removed; `next_events` stays cancel-safe through the framed write buffer (pinned by a cancellation test).
- `chunk::ChunkWriter::write` now returns `Result<Vec<u8>, RtmpError>` instead of `Vec<u8>`.
  `Error` gains a new `FieldOverflow` variant.
- **#1108 (RTMP-W7)**: `ServerSession`/`ClientSession` gain a new `peer_bandwidth: Option<u32>`
  field, tracking `SetPeerBandwidth`'s declared value separately from `ack_threshold` (see Fixed,
  below). `ClientSession` also gains `advertised_window_ack_size: u32`.
- **#1108 (RTMP-W1)**: `chunk::ChunkAssembler` gains a new `pub fn abort(&mut self, csid: u32)`,
  and `io::RtmpConnection` gains a new private `pending_write` field (both additive).
- **#1108 (RTMP-W6)**: `server::ServerEvent` gains a new `Unsupported { message_type_id: u8 }`
  variant (additive; the enum is `#[non_exhaustive]`).
- `amf0::Command::to_body` now returns `Result<Vec<u8>, RtmpError>` instead of `Vec<u8>` (it
  panicked on an over-long AMF0 string; see Fixed).

### Added
- `io::AsyncRtmpClient` (feature `tokio`): the publish client adapter (`connect`, `from_stream`, `publish`, `send_audio` / `send_video` / `send_metadata`, `next_events`), bounded by `RtmpTimeouts`; `read_idle` spans the whole wait for a non-empty batch (empty chunks do not restart it). The client adapter reads inbound traffic only in `next_events`, so a send-only caller must also drive it.
- `target::RtmpTarget` / `RtmpUrlError`: `rtmp://host[:port]/app/stream-key[?query]` parsed with the `url` crate; `tcUrl` keeps IPv6 brackets and never carries userinfo, query or fragment.
- `DEFAULT_MAX_IN_PROGRESS_BYTES` (16 MiB) and `ServerConfig::max_in_progress_bytes` /
  `ClientConfig::max_in_progress_bytes`: the per-connection budget for bytes held across
  in-progress chunk streams (see Fixed, RTMP-W1).

### Fixed
- No awaited IO in the tokio adapters is unbounded (an idle or stalled peer used to hold a connection forever).
- `tcUrl` built from a bare IPv6 address and port (`rtmp://::1:1935/live`) is now bracketed (`rtmp://[::1]:1935/live`).
- **Remote panic (release audit)**: a peer-chosen publishing name of 65 518..=65 535 bytes fit the
  request's AMF0 u16 string prefix but, echoed into the `"<key> is now published."` `onStatus`
  description, overflowed it; `Command::to_body` then hit `.expect` and panicked the server. The
  session now returns an `Err` instead. The client's internal command builders no longer
  `.expect` either (a large `app`/`tc_url`/`stream_key` is an `Err`, not a panic).
- **RTMP-W1 aggregate memory budget**: nothing bounded the total bytes held across in-progress chunk
  streams (up to 512 MiB per connection). `ChunkAssembler` now reserves each new message's full
  declared length against a per-connection budget (see Added) and releases it on
  completion or abort; exceeding it is a fatal `Malformed` error for the connection (#1085).
- **RTMP-W6 aggregate messages**: Aggregate (type 22) messages are now unpacked into their FLV-tag
  sub-messages and delivered as ordinary media events, timestamps renormalised against the
  aggregate's (§7.1.6); a truncated or back-pointer-inconsistent aggregate is `Malformed` (#1085).
- **RTMP-W9 client publish sequence**: the client now sends `releaseStream` -> `FCPublish` ->
  `createStream` (the ffmpeg/OBS/FMLE order) and announces `flashVer` in `connect` (#1085).
- **#1108 (RTMP-W2)**: `Abort` (§5.4.2) was accepted and silently ignored, so a csid's
  `in_progress` flag stayed set after the sender discarded its partial message — the next Type 3
  chunk that started a genuinely new message on that csid was instead appended to the stale
  aborted payload as a continuation. `ChunkAssembler::abort` now clears the csid's in-progress
  state, and both sessions call it on `Abort`.
- **#1108 (RTMP-W3)**: a Type 1/2 chunk header arriving while a message was already in progress
  on that csid (a header interleaved mid-message, rather than at a message boundary) silently
  reset the csid's state and discarded the in-flight bytes instead of erroring. Now rejected with
  `Malformed`.
- **#1108 (RTMP-W6)**: an AMF3-encoded command (message type 17 — a leading format-marker byte,
  then an otherwise-ordinary AMF0 command body) got no reply at all; the server now decodes it
  exactly like an AMF0 command. Data-AMF3(15)/Shared-Object(16/19) messages (still
  out of scope to decode — see the crate's non-goals) now surface a
  `ServerEvent::Unsupported { message_type_id }` event instead of being silently dropped with no
  signal at all.
- **#1108 (RTMP-W7)**: `SetPeerBandwidth` (§5.4.5, limits OUR outbound bandwidth) was misapplied
  as the ack threshold (§5.4.4's `WindowAckSize` job — how often WE acknowledge inbound bytes),
  overwriting `ack_threshold` with an unrelated value and echoing it back as our own advertised
  window. Now tracked separately in `peer_bandwidth`; the client replies with its OWN configured
  window size, only when it actually changed from what it last advertised.
- **#1108 (RTMP-W8)**: the client never answered a User Control `PingRequest` (event 6 —
  FMS/Wowza liveness probe), so a server probing this way got no `PingResponse` and could drop
  the publisher. It now replies with `PingResponse`.
- **#1108 (RTMP-W10)**: `RtmpConnection::next_events` was not cancel-safe on its write half —
  `handle_data` had already consumed the input and advanced the session's state before
  `write_all(&reply)` (itself a cancellation point) confirmed the reply was sent, so a caller
  wrapping this in `tokio::time::timeout`/`select!` could lose part of the protocol reply while
  the session believed it had gone out. The reply is now recorded in a new `pending_write` field
  synchronously (no await point) before any write attempt, flushed one `write` call at a time
  (never `write_all`, whose own partial-write count isn't recoverable after cancellation) so a
  cancelled flush leaves exactly the unsent remainder for the next call to retry.
- **#1108 (RTMP-W11)**: the C0/S0 version byte was parsed and discarded, so an RTMPE peer
  (version 6) or garbage was accepted here and only surfaced later as a confusing chunk-parse
  error. Both handshake sides now reject a version other than 3 with `Malformed`.
- `ChunkWriter::write` no longer silently truncates a 16 MiB+ (2^24) message body's 24-bit
  `message_length` field while still writing every payload byte, which misframed every later
  message on the chunk stream (#1129).
- AMF0 Object/ECMA-array key lengths and ECMA-array/strict-array/long-string counts are now
  range-checked instead of silently truncated (#1129).

## [0.6.1] - 2026-09-26

### Security
Fixes GHSA-fjrp-rx2c-c9pw and GHSA-hgmf-qpx9-6gg2.

### Fixed
- `publish`'s stream-key check now compares in constant time instead of a
  plain `!=`, so a mismatch cannot be distinguished by comparison timing.
- A connection is now closed after `MAX_FAILED_PUBLISH_ATTEMPTS` (3)
  `publish` attempts with a mismatched stream key, instead of allowing
  unlimited retries on the same connection.
- `chunk::ChunkAssembler` reassembled a chunked message by cloning the whole
  accumulated payload on every continuation chunk and compacting its input
  buffer once per chunk, so a message split into many small chunks (e.g. an
  8 MiB message at a 1-byte chunk size, reachable via an early, pre-`connect`
  Set Chunk Size) cost O(message_length²/chunk_size) instead of
  O(message_length). Each chunk's bytes are now appended in place to the
  owning chunk stream's own persistent buffer (never cloned), and the
  consumed prefix of the input buffer is compacted once per assembled
  message rather than once per chunk. `set_chunk_size` still accepts any
  peer-announced value from `1..=MAX_CHUNK_SIZE` unchanged (a peer's own
  chosen chunk size must be honoured exactly, or every later chunk boundary
  is misparsed) — this fix is the reassembly algorithm, not a stricter
  chunk-size floor.

## [0.6.0] - 2026-08-11

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
