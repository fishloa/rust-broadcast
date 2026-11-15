# rtmp-runtime 0.11.0

_Released 2026-11-15._

Minor-class release (0.10 -> 0.11). RTMP's wire identities (chunk stream id, message stream id, timestamp, message type id) are typed instead of bare integers, the adapter has one config shape per role, and a peer that announces a chunk size above the engine ceiling now fails the session instead of silently corrupting the stream. A non-canonical AMF0 command no longer breaks parsing, and an FLV header can no longer be written with a `DataOffset` that contradicts its own bytes.

Version: 0.10.0 -> 0.11.0.

## Breaking changes and migration

### Wire identities are typed

`chunk::BasicHeader::chunk_stream_id`, `chunk::Message::chunk_stream_id` and `ProtocolControl::Abort`'s field are `ChunkStreamId` (2..=65599, `basic_header_encodable()`); `chunk::Message::message_stream_id` and the Type 0 header's field are `MessageStreamId`; `chunk::Message::timestamp` and the Type 0/1/2 timestamp fields are `Timestamp` (a span carried as whole milliseconds) with `core::time::Duration` deltas; `ProtocolControl::Acknowledgement` wraps a `SequenceNumber`; `ChunkAssembler::abort` takes a `ChunkStreamId`. The `message::CONTROL_CHUNK_STREAM_ID`/`CONTROL_MESSAGE_STREAM_ID` constants are typed. A chunk stream id can no longer be passed where a message stream id belongs. Wire bytes are unchanged.

```rust
// before
let message = Message { chunk_stream_id: 4, timestamp: 0, .. };
let millis: u32 = message.timestamp;
assembler.abort(4);
// after
let message = Message {
    chunk_stream_id: ChunkStreamId::new(4),
    timestamp: Timestamp::ZERO,
    ..message
};
let millis: u32 = message.timestamp.millis();
assembler.abort(ChunkStreamId::new(4));
```

### The message type id is a typed enum

`MessageTypeId` (`SetChunkSize`, `Abort`, `Acknowledgement`, `UserControl`, `WindowAckSize`, `SetPeerBandwidth`, `Audio`, `Video`, `DataAmf3`, `CommandAmf3`, `DataAmf0`, `CommandAmf0`, `Aggregate`, `Other(u8)`, `#[non_exhaustive]`, with `from_u8`/`to_u8`, `name()` and `Display`) replaces a raw `u8` in `Message::message_type_id`, the Type 0/1 header field, `ProtocolControl::message_type_id()`/`from_payload()` and `ServerEvent::Unsupported::message_type_id`. The `message::msg_type` constants module is private.

```rust
// before
let message = Message { message_type_id: msg_type::VIDEO, .. };
if message.message_type_id == msg_type::AUDIO { .. }
ProtocolControl::from_payload(msg_type::SET_CHUNK_SIZE, &payload)
// after
let message = Message { message_type_id: MessageTypeId::Video, .. };
if message.message_type_id == MessageTypeId::Audio { .. }
ProtocolControl::from_payload(MessageTypeId::SetChunkSize, &payload)
let from_wire = MessageTypeId::from_u8(byte);
```

### Adapter configs are one shape per role

The sans-IO `client::ClientConfig` is `client::RtmpClientConfig` and `server::ServerConfig` is `server::RtmpServerConfig`. The tokio adapter gains `io::RtmpClientConfig { target, timeouts }` and `io::RtmpServerConfig { session, timeouts }`, and its constructors take them by value instead of `(target, timeouts)` and an `impl Into` session.

```rust
// before
let client = AsyncRtmpClient::connect(&target, timeouts).await?;
let client = AsyncRtmpClient::from_stream(stream, &target, timeouts);
let server = AsyncRtmpServer::bind(addr, server_config).await?.with_timeouts(t);
let conn = RtmpConnection::from_stream_erased(stream, ServerSession::new(cfg), t);
// after
let client = AsyncRtmpClient::connect(io::RtmpClientConfig::new(target).with_timeouts(timeouts)).await?;
let client = AsyncRtmpClient::from_stream(stream, io::RtmpClientConfig::new(target).with_timeouts(timeouts));
let server = AsyncRtmpServer::bind(addr, io::RtmpServerConfig::default().with_session(server_config).with_timeouts(t)).await?;
let conn = RtmpConnection::from_stream(stream, io::RtmpServerConfig::default().with_session(cfg).with_timeouts(t));
```

`RtmpConnection::from_stream_erased` and the generic `RtmpConnection<S>` form are gone: a connection is always the erased `RtmpConnection`.

### `ChunkSize` is a validated newtype

`ProtocolControl::SetChunkSize` wraps `ChunkSize` (1..=0x7FFF_FFFF), `to_message` is total and `try_to_message` is removed. `ChunkAssembler`/`ChunkWriter::set_chunk_size` and `ChunkAssembler::with_chunk_size` take a `ChunkSize`. A peer's `SetChunkSize` above the engine ceiling (`chunk::MAX_CHUNK_SIZE`, 16 MiB) used to be accepted with the size silently clamped, so every following chunk boundary was parsed at a size the peer never announced; it is now rejected and the session fails with `RtmpError::Malformed` instead of corrupting the stream.

```rust
// before
assembler.set_chunk_size(n);
// after
assembler.set_chunk_size(ChunkSize::new(n)?);
```

### `target::RtmpTarget::tc_url` is derived

It is a private field read with `RtmpTarget::tc_url()`, computed from `host`/`port`/`tls`/`app`, so it can never disagree with them.

```rust
// before
let url = &target.tc_url;
// after
let url = target.tc_url();
```

### The error type names the failure

`RtmpError`'s three stringly catch-alls (`Malformed { what }`, `Unsupported { what }`, `UnexpectedState { what }`) are gone for every non-wire failure: a caller branches on `ChunkSizeOutOfRange`, `InvalidLimitType`, `UnsupportedUserControlEvent`, `UnsupportedAmf0Marker`, `Amf0DepthExceeded`, `Amf0StringTooLong`, `Amf0DateTimeZone`, `Amf0Utf8`, `CommandNameNotString`, `CommandTransactionIdNotNumber`, `HandshakeVersion`, `TooManyChunkStreams`, `InputBudgetExceeded` and `ChunkStreamState`, each carrying the value or stream id that failed. `Malformed` stays only for "these bytes are not that structure". `RtmpError` derives `PartialEq` but no longer `Eq` (two variants carry an `Amf0Value`). `target::RtmpUrlError::Parse` wraps `url::ParseError` and `RtmpUrlError::Scheme` wraps the new `target::Scheme`.

```rust
// before
if matches!(err, RtmpError::Unsupported { ref what } if what == "chunk size") { .. }
// after
if let RtmpError::ChunkSizeOutOfRange { .. } = err { .. }
```

### `FlvHeader` derives its `DataOffset`

`flv::FlvHeader` no longer stores a `data_offset` field: the `DataOffset` it writes is derived from `FLV_HEADER_LEN + extra.len()` (§E.2) and read with `FlvHeader::data_offset()`. A header whose field disagreed with its `extra` bytes used to serialize to an offset the parser rejects.

```rust
// before
let offset = header.data_offset;   // a stored field
// after
let offset = header.data_offset(); // derived from the extra bytes
```

### Durations are `Duration`

`UserControl::SetBufferLength`'s `buffer_ms: u32` is `buffer: Duration` (wire unchanged: whole milliseconds, with `RtmpError::FieldOverflow` instead of a clamped value), `RtmpClientConfig::play_buffer` is a `Duration` (default `DEFAULT_PLAY_BUFFER`), and `ClientSession::send_audio`/`send_video` plus the adapter's `send_audio`/`send_video`/`encode_audio`/`encode_video` take the message `Timestamp` instead of a `u32` millisecond count.

```rust
// before
let mut out = session.send_video(&data, pts_ms)?;
// after
let mut out = session.send_video(Timestamp::from_millis(pts_ms), &data)?;
```

## Fixes

- `Command::parse` takes each argument's length from the parser that read it instead of re-serializing the parsed value and measuring that. The two agree only while every AMF0 encoding is canonical (any non-zero Boolean byte decodes to `true` but re-serializes to `0x01`), so a non-canonical argument could shift every following boundary and fail the command.
- The acknowledgement now propagates the writer's error instead of `.expect(..)`, and a committed-but-missing chunk-stream entry or an fmt/variant mismatch is reported as `RtmpError::ChunkStreamState` instead of `unreachable!()`.

## Added

- `io::RtmpClientConfig` and `io::RtmpServerConfig` (see above), so a host maps its timeout config once per role.
- `ChunkAssembler::feed`, `next_message`, `compact`, `pending_len` and `swap_pending`: the incremental interface for a caller that applies a Set Chunk Size between messages, and `chunk::DEFAULT_CHUNK_SIZE` is a `ChunkSize`.

## Dependencies

- `broadcast-common` 12 -> 13 (published first, from tag `broadcast-common-v13.0.0`).

---

Published from tag `rtmp-runtime-v0.11.0`.
