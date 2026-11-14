# Changelog

All notable changes to `rtmp-runtime` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- BREAKING: `RtmpClientConfig::play_buffer` is a `core::time::Duration` (was `play_buffer_ms: u32`) and the default constant is `DEFAULT_PLAY_BUFFER` (was `DEFAULT_PLAY_BUFFER_MS`); the value still goes on the wire as whole milliseconds in SetBufferLength (§6.7) (#2693).

#### Added
- `io::RtmpClientConfig { target, timeouts }` (`new(target)`, `with_timeouts`) and `io::RtmpServerConfig { session, timeouts }` (`Default`, `with_session`, `with_timeouts`): the same config shape as the RTSP and SRT adapters, so a host maps its timeout config once per role (#2660).

#### Removed
- BREAKING: `AsyncRtmpClient::connect(target, timeouts)` and `AsyncRtmpClient::from_stream(stream, target, timeouts)`; both take an `io::RtmpClientConfig`. Migration: `connect(&target, t)` -> `connect(RtmpClientConfig::new(target).with_timeouts(t))`, `from_stream(stream, &target, t)` -> `from_stream(stream, RtmpClientConfig::new(target).with_timeouts(t))`.
- BREAKING: `AsyncRtmpServer::bind(addr, ServerConfig).with_timeouts(t)`; `bind` takes an `io::RtmpServerConfig`. Migration: `bind(addr, RtmpServerConfig::default().with_session(cfg).with_timeouts(t))`. `with_timeouts` on the bound server is gone.
- BREAKING: `RtmpConnection::from_stream(stream, session, timeouts)` (the generic form) and `RtmpConnection::from_stream_erased`: one `RtmpConnection::from_stream(stream, RtmpServerConfig)` erases the stream type, as the server and multimux already used. Migration: `from_stream_erased(s, ServerSession::new(cfg), t)` -> `from_stream(s, RtmpServerConfig::default().with_session(cfg).with_timeouts(t))`. A connection is now always `RtmpConnection` (the erased form), not `RtmpConnection<S>` for a named `S`.

#### Changed
- BREAKING: the message type id is a typed `message::MessageTypeId` (`SetChunkSize`, `Abort`, `Acknowledgement`, `UserControl`, `WindowAckSize`, `SetPeerBandwidth`, `Audio`, `Video`, `DataAmf3`, `CommandAmf3`, `DataAmf0`, `CommandAmf0`, `Aggregate`, `Other(u8)`, `#[non_exhaustive]`, with `from_u8`/`to_u8`, `name()` and `Display`) wherever a raw `u8` stood for one: `Message::message_type_id`, `MessageHeader::Type0`/`Type1`'s field, `ProtocolControl::message_type_id()`/`from_payload()`, `ServerEvent::Unsupported::message_type_id`. The `message::msg_type` constants module is private. Migration: `message_type_id: msg_type::VIDEO` -> `message_type_id: MessageTypeId::Video`, `== msg_type::AUDIO` -> `== MessageTypeId::Audio`, `from_payload(msg_type::SET_CHUNK_SIZE, ..)` -> `from_payload(MessageTypeId::SetChunkSize, ..)`, and a byte from the wire -> `MessageTypeId::from_u8(byte)` (#2666).
- BREAKING: `client::ClientConfig` is `client::RtmpClientConfig` and `server::ServerConfig` is `server::RtmpServerConfig`, matching `RtspClientConfig`/`RtspServerConfig`/`HlsClientConfig`. Two modules can now hold a `RtmpClientConfig` (the sans-IO session's and the tokio adapter's) without a name clash; they configure different layers.
- BREAKING: `ProtocolControl::SetChunkSize` now wraps a validated `ChunkSize` newtype (1..=0x7FFF_FFFF); `to_message` is total and `try_to_message` is removed (#2659).

## Changed
- Narrowed the `tokio` dev-dependency from `features = ["full"]` to the features the tests use (`net`, `io-util`, `rt`, `rt-multi-thread`, `macros`, `time`, `sync`, `test-util`).

## [0.10.0] - 2026-11-14
### Removed
- BREAKING: `io::RtmpTimeouts`; the adapters take `broadcast_common::timeouts::Timeouts` (the same four fields and defaults), shared with the RTSP and SRT adapters (#2514).

### Changed
- BREAKING: `RtmpTarget::stream_key` and `app` are percent-decoded (they were raw), and `AsyncRtmpClient::connect` refuses an `rtmps` target (plain TCP to a TLS port would leak the key; use `from_stream` over TLS) (#2588).
- **BREAKING**: `RtmpConnection`'s default stream type is now the erased `BoxStream` (a TCP socket, a TLS stream over one, or any other byte stream), so a caller naming one `RtmpConnection` no longer names its transport. `AsyncRtmpServer::accept` returns the new `AcceptedRtmpConnection` (the publisher's address, which an erased stream cannot report, plus the connection) instead of a `RtmpConnection`; call `into_connection()` for the connection itself. `RtmpConnection::peer_addr` is removed; use `AcceptedRtmpConnection::peer_addr`.
- rtmp-runtime no longer dev-depends on `transmux`; the tests that decoded its FLV output with `transmux::FlvDemux` moved into transmux's test suite (#2339).

### Added
- `target::RtmpTarget` accepts `rtmps://` (`RtmpTarget::tls`, default port `RTMPS_DEFAULT_PORT` 443, `SCHEME_RTMP`/`SCHEME_RTMPS`), percent-decodes the app and stream-key path (RFC 3986 §2.1; the query stays as written), and refuses a key that is only a query (`RtmpUrlError::PathNotUtf8` for undecodable text). A new `target` feature (implied by `tokio`) exposes the URL grammar without sockets (#2588).
- `io::RtmpStream`, `io::BoxStream` and `RtmpConnection::from_stream_erased`: wrap any connected stream, so a session can be driven over something other than a TCP socket (an in-memory pipe in a test, a TLS stream in a host) without a second connection type.
- `flv::FlvTagHeader` (`parse`, `total_len`, `Serialize`): the 11-byte FLV tag header on its own, so an incremental reader can size a tag before its body arrives. `FlvTag::parse` and the tolerant walk decode the header through it, and `transmux`'s streaming FLV demuxer uses it instead of hand-rolled `u32::from_be_bytes` reads.
- `amf0::amf0_values(&[Amf0Value]) -> Result<Vec<u8>>`: encode several bare AMF0 values as one message body (the shape an FLV `onMetaData` script tag has). `transmux`'s FLV muxer writes its script tag through it.
- `rtmp_runtime::flv`: the Adobe FLV v10.1 Annex E header and tag codec (`FlvHeader`, `TagType`, `FlvTag<'a>`, the strict `tags()` walk, `Parse`/`Serialize` with byte-exact round trips on the committed FLV fixtures), `no_std` + `alloc`. It replaces the crate-private FLV writer the server and client used, and transmux frames FLV through it, so the workspace has one FLV tag codec (#2342). A final tag whose trailing `PreviousTagSize` is cut off is accepted (the field is redundant, §E.4.1); a cut tag body is an error. New fuzz target `rtmp_flv_tags`.
- `rtmp_runtime::flv::tags_resync(input, header) -> ResyncWalk`: the tolerant walk beside the strict `tags()`, tested in `tests/it/flv_tags.rs` over the committed FLV captures (a damaged tag is skipped and its offset reported, a cut-off final tag is flagged, a header with no tags yields none). It keeps the tags after a damaged span (skipping a span whose tag chain does not hold to the next consistent one, reported in `damaged`), reports a final tag whose body overruns the input as `truncated_tail` instead of dropping the good prefix, and accepts a final tag missing only its redundant trailing `PreviousTagSize`. It is the workspace's one FLV tag walker: `transmux`'s `FlvDemux` adapts its result, and transmux's own resync logic is gone.
- The sans-IO core (handshake, chunk stream, AMF0, messages, client and server sessions) builds `no_std` + `alloc`; only the `tokio` adapter needs `std`. This lets `transmux` depend on rtmp-runtime for its RTMP spokes (#2339).
- `ChunkAssembler::feed` and `ChunkAssembler::next_message` are public: the incremental form a caller needs when one input buffer carries a Set Chunk Size followed by chunks framed at the new size (`push` uses one chunk size for the whole buffer).

### Fixed
- `flv::FlvHeader` serialize now writes the `DataOffset - 9` extra header bytes the parser skipped, so a non-standard `DataOffset` round-trips byte-exactly; `FlvTag::parse` rejects a trailing `PreviousTagSize` that disagrees with `11 + DataSize`.
- A Data-AMF0 message that begins with the AMF0 string `@setDataFrame` is forwarded as an FLV script tag that begins with `onMetaData`; the publisher-to-server handler name used to be forwarded verbatim as the script tag name.
- `ServerSession` refuses a second `connect` while publishing with `RtmpError::UnexpectedState` and keeps publishing; it used to demote the session to `Connected` and silently drop all following media (bug audit E).
- `BasicHeader` and `MessageHeader` `serialize_into` returned `RtmpError::BufferTooShort` for a too-small output buffer; they now return `RtmpError::OutputBufferTooSmall` (#2269).

### Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12.

## [0.9.0] - 2026-11-07
### Changed
- Error enums derive `Clone`, `PartialEq` and `Eq` wherever every field allows it (c6 C6-14).
- BREAKING: error variant `RtmpError::BufferTooShort` now wraps `broadcast_common::error::BufferTooShort` (same `need`/`have`/`what`; the message text is the shared one).
- Length pre-checks go through `broadcast_common::len::{need_in, need_out}`; the error values are unchanged (c3 S1).
- Server and client connection adapters share one private `Driver` and `SessionCodec` over an `RtmpSession` trait; no public API change (c3 R1).
- Dropped the unused `log` dependency (c2 TH4).
### Removed
- BREAKING: Unused `ServerConfig::with_window_ack_size` and `with_peer_bandwidth` (c2 TH1).
### Dependencies
- Sibling crates move to a new epoch in this release wave: `broadcast-common` 10 -> 11. The `Cargo.toml` requirements name the new versions; no other manifest change.

## [0.8.1] - 2026-11-03
### Added
- **Play mode for the sans-IO client** (issue #1460): `ClientConfig::mode` (`ClientMode::{Publish, Play}`, default `Publish`) and `ClientConfig::play_buffer_ms` (default `DEFAULT_PLAY_BUFFER_MS`). In play mode `ClientSession` runs `connect` -> `createStream` -> `SetBufferLength` + `play` (Adobe RTMP 1.0 §7.2.2.1, §6.7) with the stream name in `stream_key`, and reports `ClientEvent::Playing` (`NetStream.Play.Start`), `ClientEvent::Media { flv }` (Audio/Video/Data-AMF0 and unpacked Aggregate messages as FLV tags, the first chunk prefixed with the FLV file header, exactly as `ServerEvent::Media`), `ClientEvent::StreamEnded` (Stream EOF or `NetStream.Play.Stop`/`Complete`) and, as `ClientEvent::Error`, `NetStream.Play.StreamNotFound`, `NetStream.Play.Failed` and any `onStatus` of level `error`. Set Chunk Size, Window Acknowledgement Size/Acknowledgement and PingRequest/PingResponse behave as on the publish side; AMF3 command messages are decoded after their marker byte. Publish mode is unchanged.
### Changed
- The FLV tag writer and the Aggregate (22) unpacking moved from `server.rs` into a crate-private `flv_bridge` module shared by the server and the client; no behaviour change.
- Builds against `transmux` 0.29 (was 0.28); no API change beyond the version.
### Documentation
- Fixed documentation links/citations to resolve to existing paths (docs/tools hygiene).

## [0.8.0] - 2026-10-17
### Documentation
- README restructured: what the crate does and does not do, a standards-supported table (edition, scope, status), feature-flag table, a doctested quick start and related crates; crate-root docs now render the README (or are kept in step with it).

### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `transmux` 0.26 -> 0.27). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

### Changed
- `UserControl::to_message` / `ProtocolControl::to_message` keep their documented panic via an explicit `expect` message (use `try_to_message` for untrusted values).

### Security (secrets and identity handling, #1142)

- `Debug` of `ServerConfig`, `ClientConfig`, `RtmpTarget`, `ServerEvent` (and therefore `ServerSession`) no longer prints the stream key; `Media` prints its length, not its bytes.
- The stream-key comparison uses `subtle::ConstantTimeEq` (new `subtle` dependency) instead of a hand-written fold.
- Documented: `ServerConfig::expected_stream_key = None` (the default) admits any publisher; deployments reachable from untrusted networks must set a key. New `tests/secret_hygiene.rs`, `tests/secret_debug_guard.rs`.

### Fixed

- **RTMP `message length` over 24 bits is rejected (#1129).** `MessageHeader::{Type0,Type1}` wrote the caller-supplied `u32` with the high byte silently dropped; `serialize_into` now returns `RtmpError::FieldOverflow`.

### Fixed
- `ServerSession::handle_data` no longer copies every post-handshake read into a scratch `Vec` before the chunk assembler buffers it (one copy instead of two; measured 246 976 -> 186 976 bytes for a 60 000-byte read) (#1085/RTMP-O1). `test-alloc` added as a dev-dependency for the allocation-count test.

## [0.7.2] - 2026-10-13
### Added

- `ProtocolControl::try_to_message` (fallible `to_message`).

### Fixed

- `ClientSession::send_metadata` and the server's connect reply (configured `chunk_size` of 0) return `Err` instead of panicking on an unserializable value.

## [0.7.1] - 2026-10-11
### Fixed
- `chunk::ChunkWriter::write` returns the new `RtmpError::InvalidChunkStreamId { chunk_stream_id }` for a `Message::chunk_stream_id` outside `2..=65599` instead of panicking. Not breaking: the signature is unchanged and `RtmpError` is `#[non_exhaustive]`.

## [0.7.0] - 2026-10-05

### Security
Includes fixes for privately reported security issues; no public advisory is published.

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
  the session believed it had gone out. The reply is now queued in the `Framed` codec's write buffer
  (see Changed: `RtmpConnection` is a `Framed` over the sans-IO session) before any flush is
  attempted, so a cancelled flush leaves exactly the unsent remainder queued for the next call
  to retry.
- **#1108 (RTMP-W11)**: the C0/S0 version byte was parsed and discarded, so an RTMPE peer
  (version 6) or garbage was accepted here and only surfaced later as a confusing chunk-parse
  error. Both handshake sides now reject a version other than 3 with `Malformed`.
- `ChunkWriter::write` no longer silently truncates a 16 MiB+ (2^24) message body's 24-bit
  `message_length` field while still writing every payload byte, which misframed every later
  message on the chunk stream (#1129).
- AMF0 Object/ECMA-array key lengths and ECMA-array/strict-array/long-string counts are now
  range-checked instead of silently truncated (#1129).

### Added
- `AsyncRtmpClient` gains sans-IO `encode_video`/`encode_audio`/`encode_metadata`
  (returning the chunk-stream-framed message bytes without writing) and
  `write_frame` (writing one already-framed message under `RtmpTimeouts::write`),
  so a caller driving its own egress can frame and write separately.
  `send_video`/`send_audio`/`send_metadata` now delegate to the `encode_*` half.

- `io::AsyncRtmpClient` (feature `tokio`): the publish client adapter (`connect`, `from_stream`, `publish`, `send_audio` / `send_video` / `send_metadata`, `next_events`), bounded by `RtmpTimeouts`; `read_idle` spans the whole wait for a non-empty batch (empty chunks do not restart it). The client adapter reads inbound traffic only in `next_events`, so a send-only caller must also drive it.
- `target::RtmpTarget` / `RtmpUrlError`: `rtmp://host[:port]/app/stream-key[?query]` parsed with the `url` crate; `tcUrl` keeps IPv6 brackets and never carries userinfo, query or fragment.
- `DEFAULT_MAX_IN_PROGRESS_BYTES` (16 MiB) and `ServerConfig::max_in_progress_bytes` /
  `ClientConfig::max_in_progress_bytes`: the per-connection budget for bytes held across
  in-progress chunk streams (see Fixed, RTMP-W1).

### Changed (breaking)
- `RtmpConnection` is now `RtmpConnection<S = TcpStream>` (a `tokio_util::codec::Framed` over the sans-IO session) with `RtmpConnection::from_stream(stream, session, RtmpTimeouts)`; `AsyncRtmpServer` gains `with_timeouts`. New `io::RtmpTimeouts` (connect / handshake / read_idle / write; defaults 10 s / 10 s / 30 s / 10 s): a deadline expiry from `next_events` is `io::ErrorKind::TimedOut` and closes the connection. The `pending_write` field is removed; `next_events` stays cancel-safe through the framed write buffer (pinned by a cancellation test).
- `chunk::ChunkWriter::write` now returns `Result<Vec<u8>, RtmpError>` instead of `Vec<u8>`.
  `Error` gains a new `FieldOverflow` variant.
- **#1108 (RTMP-W1)**: `chunk::ChunkAssembler` gains a new `pub fn abort(&mut self, csid: u32)`,
  (additive).
- **#1108 (RTMP-W6)**: `server::ServerEvent` gains a new `Unsupported { message_type_id: u8 }`
  variant (additive; the enum is `#[non_exhaustive]`).
- `amf0::Command::to_body` now returns `Result<Vec<u8>, RtmpError>` instead of `Vec<u8>` (it
  panicked on an over-long AMF0 string; see Fixed).

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
  capture `../fixtures/rtmp/obs-publish.bin` (a real `ffmpeg -f flv` publish).
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
