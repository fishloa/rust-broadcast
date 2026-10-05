# rtmp-runtime 0.7.0

_Released 2026-10-05._

Breaking (0.x minor) release that is also the first publication of everything since 0.6.0: it carries the never-published 0.6.1 security fixes, a release-audit remote-panic fix, the run-09/#1108 protocol fixes (RTMP-W1 to W11), and the de-hand-roll wave's new socket adapters. **Upgrade if you accept RTMP from publishers you do not control.** Source-level breaks are small: `chunk::ChunkWriter::write` and `amf0::Command::to_body` now return `Result`, `RtmpError` gains `FieldOverflow`, and the tokio-adapter `RtmpConnection` becomes generic over its stream. The latest published version before this one is 0.6.0; 0.6.1 was prepared but never tagged, so its fixes ship here. Behaviour is the larger change: a connection is now closed on deadline expiry, on a mismatched-key retry limit, on exceeding a per-connection memory budget, and on malformed framing that used to be tolerated.

## Security (from the unpublished 0.6.1)

- **GHSA-fjrp-rx2c-c9pw**: `ChunkAssembler` copied the whole message received so far on every continuation chunk, so reassembly cost grew with the square of the message length. A peer could announce chunk size 1 before `connect` and send one large message to burn CPU. Each chunk is now appended in place and the input buffer compacted once per assembled message; an 8 MiB message at chunk size 1 is linear. Peer-announced chunk sizes `1..=MAX_CHUNK_SIZE` are still honoured exactly.
- **GHSA-hgmf-qpx9-6gg2**: `publish` compared the stream key with a plain `!=`, which leaks the key through timing, and allowed unlimited failed attempts on one connection. The check is now constant-time and the connection is closed after `MAX_FAILED_PUBLISH_ATTEMPTS` (3) mismatched `publish` attempts.

## Remote panic (release audit)

A peer-chosen publishing name of 65 518 to 65 535 bytes fit the request's AMF0 u16 string prefix but overflowed it when echoed into the `"<key> is now published."` `onStatus` description; `Command::to_body` hit an `.expect` and panicked the server. The session now returns an `Err`. The client's internal command builders no longer `.expect` either, so an oversized `app`, `tc_url` or `stream_key` is an `Err`, not a panic.

## Breaking changes

### `chunk::ChunkWriter::write` and `amf0::Command::to_body` return `Result`

```rust
// 0.6.0
let bytes: Vec<u8> = writer.write(&msg);
let body: Vec<u8> = command.to_body();
// 0.7.0
let bytes: Vec<u8> = writer.write(&msg)?;     // Result<Vec<u8>, RtmpError>
let body: Vec<u8> = command.to_body()?;       // Result<Vec<u8>, RtmpError>
```

`RtmpError` (`#[non_exhaustive]`) gains `FieldOverflow(broadcast_common::len::FieldOverflow)`. `ChunkWriter::write` used to truncate a message of 16 MiB or more (2^24) in its 24-bit `message_length` field while still writing every payload byte, which misframed every later message on the chunk stream (#1129). AMF0 object and ECMA-array key lengths, and ECMA-array, strict-array and long-string counts, are now range-checked instead of truncated (#1129).

### Tokio adapter: `RtmpConnection<S = TcpStream>` and timeouts

`io::RtmpConnection` is now `RtmpConnection<S = TcpStream>`, a `tokio_util::codec::Framed` over the sans-IO session. `AsyncRtmpServer::accept` still returns `RtmpConnection` (the default `TcpStream`), so code naming the type is unaffected; the new `RtmpConnection::from_stream(stream, session, RtmpTimeouts)` lets you drive any stream. `AsyncRtmpServer` gains `with_timeouts(RtmpTimeouts)`.

New `io::RtmpTimeouts { connect, handshake, read_idle, write }` with defaults 10 s, 10 s, 30 s, 10 s and `with_connect`/`with_handshake`/`with_read_idle`/`with_write` builders. A deadline expiry from `next_events` is an `io::Error` of kind `TimedOut` and closes the connection. Protocol errors keep kind `InvalidData`. **No awaited IO in the adapters is unbounded any more**: an idle or stalled peer used to hold a connection forever, so a deployment that deliberately kept long-idle publishers connected must raise `read_idle`. `next_events` stays cancel-safe through the framed write buffer (pinned by a cancellation test).

## New capabilities

- **`io::AsyncRtmpClient`** (feature `tokio`): the publish client adapter: `connect`, `from_stream`, `publish`, `send_audio`, `send_video`, `send_metadata`, `next_events`, all bounded by `RtmpTimeouts`. The client reads inbound traffic only in `next_events`, so a send-only caller must also drive it. Sans-IO halves `encode_audio`, `encode_video`, `encode_metadata` return the chunk-framed bytes without writing, and `write_frame` writes one already-framed message under `RtmpTimeouts::write`; `send_*` delegate to the `encode_*` half.
- **`target::RtmpTarget` / `RtmpUrlError`** (feature `tokio`): `rtmp://host[:port]/app/stream-key[?query]` parsed with the `url` crate. `tcUrl` keeps IPv6 brackets and never carries userinfo, query or fragment. A `tcUrl` built from a bare IPv6 address and port (`rtmp://::1:1935/live`) is now bracketed (`rtmp://[::1]:1935/live`).
- **Aggregate messages (RTMP-W6, #1085).** Aggregate (type 22) messages are unpacked into their FLV-tag sub-messages and delivered as ordinary media events with timestamps renormalised per §7.1.6. A truncated or back-pointer-inconsistent aggregate is `Malformed`.
- **Memory budget (RTMP-W1, #1085).** New `chunk::DEFAULT_MAX_IN_PROGRESS_BYTES` (16 MiB), `ServerConfig::max_in_progress_bytes` (builder `with_max_in_progress_bytes`) and `ClientConfig::max_in_progress_bytes`: the per-connection budget for bytes held across in-progress chunk streams (previously up to 512 MiB per connection). Each new message's declared length is reserved against it and released on completion or abort; exceeding it is a fatal `Malformed` error for the connection.
- `ChunkAssembler::abort(csid)` and `ServerEvent::Unsupported { message_type_id: u8 }` (additive; `ServerEvent` is `#[non_exhaustive]`).

## Behaviour changes (not API breaks)

- **Client publish sequence (RTMP-W9, #1085).** The client now sends `releaseStream`, `FCPublish`, `createStream` (the ffmpeg/OBS/FMLE order) and announces `flashVer` in `connect`. A server fixture that asserted the old order will see different commands.
- **Handshake version byte (#1108, RTMP-W11).** C0/S0 is no longer discarded: both sides reject a version other than 3 with `Malformed`, so an RTMPE peer (version 6) or garbage fails at the handshake instead of as a confusing chunk-parse error.
- **Mid-message Type 1/2 chunk header (RTMP-W3).** A Type 1 or Type 2 header arriving while a message is in progress on that csid used to silently reset the csid and discard the in-flight bytes; it is now rejected with `Malformed`.

## Fixes

- **#1108 (RTMP-W2)**: `Abort` (§5.4.2) was ignored, so the next Type 3 chunk that began a new message on that csid was appended to the stale aborted payload. `ChunkAssembler::abort` clears the csid's state and both sessions call it on `Abort`.
- **#1108 (RTMP-W6)**: an AMF3-encoded command (message type 17) got no reply; it is now decoded like an AMF0 command. Data-AMF3 (15) and Shared-Object (16, 19) messages, still out of scope to decode, surface `ServerEvent::Unsupported` instead of being dropped silently.
- **#1108 (RTMP-W7)**: `SetPeerBandwidth` (§5.4.5, limits our outbound bandwidth) was applied as the ack threshold (§5.4.4's job), overwriting `ack_threshold` and echoing it back as our advertised window. It is now tracked separately; the client replies with its own configured window size, only when that changed from what it last advertised.
- **#1108 (RTMP-W8)**: the client never answered a User Control `PingRequest` (event 6, the FMS/Wowza liveness probe), so such a server could drop the publisher. It now replies with `PingResponse`.
- **#1108 (RTMP-W10)**: `RtmpConnection::next_events` was not cancel-safe on its write half; a caller wrapping it in `timeout` or `select!` could lose part of a reply while the session believed it was sent. Fixed by the framed adapter.

## Dependencies

Verified from the `Cargo.toml` diff against `rtmp-runtime-v0.6.0` (all new ones are optional behind `tokio`):

```toml
broadcast-common = { version = "9.3" -> "9.4", default-features = false }
tokio        = { features = ["net", "io-util", "rt", "macros", "sync"] -> + "time" }
tokio-util   = { version = "0.7", optional = true, default-features = false, features = ["codec"] }   # new
futures-util = { version = "0.3", optional = true, default-features = false, features = ["sink", "std"] }  # new
bytes        = { version = "1", optional = true }   # new
url          = { version = "2", optional = true }   # new
tokio = ["dep:tokio"] -> ["dep:tokio", "dep:tokio-util", "dep:futures-util", "dep:bytes", "dep:url"]
# dev-dependencies: transmux "0.24" -> "0.25"; tokio gains ["full", "test-util"]
```

MSRV 1.95.0. The sans-IO core (no `tokio` feature) gains none of the new dependencies.

---

Published from tag `rtmp-runtime-v0.7.0`.
