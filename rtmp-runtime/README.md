# rtmp-runtime

Sans-IO **RTMP 1.0 ingest** (publish) session engine for Adobe's Real-Time
Messaging Protocol, covering both ends of a publish exchange:

- **`server`** — the ingest server side, for receiving a live push from an
  encoder or OBS.
- **`client`** — the publish client side, for pushing a live stream to a
  remote RTMP server.

`rtmp-runtime` owns the RTMP wire layer and session state machine — handshake,
chunk-stream (de)assembly, AMF0 command routing, and the publish flow. The
server hands the received media off as **FLV bytes** for a container demuxer
(e.g. [`transmux`](https://crates.io/crates/transmux)) to turn into samples;
the client accepts already-encoded audio/video/metadata to send. Both roles
are **publish-only**; egress/`play` (on either the client or server side) is
out of scope.

## Design

A **bytes-in → (bytes-out, events)** engine, with no I/O of its own:

```rust
use rtmp_runtime::server::{ServerSession, ServerEvent};

let mut session = ServerSession::with_defaults();
// Feed inbound TCP bytes; get bytes to write back + typed events.
let (reply, events) = session.handle_data(&inbound)?;
socket.write_all(&reply)?;
for ev in events {
    match ev {
        ServerEvent::Connected { app } => { /* NetConnection app name */ }
        ServerEvent::Publish { stream_key, .. } => { /* publish started */ }
        ServerEvent::Media { flv } => { /* FLV bytes → transmux::FlvDemux / StreamingFlvDemux */ }
        ServerEvent::Eof => break,
        _ => {}
    }
}
```

`handle_data` drives the whole exchange internally and buffers partial input
across calls, so it runs against any transport. With the optional **`tokio`**
feature, `io::AsyncRtmpServer` binds a listener and drives one `ServerSession`
per connection.

## What's implemented

- **Handshake** (§2): C0/S0, C1/S1, C2/S2 (simple handshake — interoperates with
  ffmpeg/OBS `-f flv` publishers).
- **Chunk stream** (§3): basic header (1/2/3-byte csid) + message header
  fmt 0/1/2/3 + extended timestamp; incremental `ChunkAssembler` (header
  inheritance, `SetChunkSize` tracking, partial-input buffering) + `ChunkWriter`.
- **Protocol control** (§4): SetChunkSize / Abort / Acknowledgement /
  WindowAckSize / SetPeerBandwidth; **User Control** events (§5, StreamBegin …).
- **AMF0** (§8): the value types + `Command` encode/decode for the ingest command
  set. (AMF3 is out of scope.)
- **Publish session** (§7): `connect` → `createStream` → `publish` (+ tolerated
  OBS extras `releaseStream`/`FCPublish`), with WindowAckSize / SetPeerBandwidth /
  SetChunkSize / `_result` / StreamBegin / `onStatus` replies, `Acknowledgement`
  accounting, and optional `expected_stream_key` gating. Audio/Video/Data messages
  are emitted as **FLV** (`ServerEvent::Media`) — concatenating them yields a valid
  FLV stream for `transmux`.
- **`tokio` adapter**: `io::AsyncRtmpServer` (listener) / `RtmpConnection`
  (feature `tokio`); the sans-IO core needs no runtime.
- **Publish client** (§7): `client::ClientSession` drives the client-side
  handshake and auto-advances `connect` → `createStream` → `publish`, then
  offers `send_audio`/`send_video`/`send_metadata` once publishing. Sans-IO
  only — there is no `tokio` adapter for it yet, unlike the server side.

Every wire structure implements symmetric `Parse`/`Serialize` with byte-identical
round-trips, verified against a real ffmpeg RTMP publish capture
(`tests/fixtures/obs-publish.bin`).

## Spec

Adobe *Real-Time Messaging Protocol (RTMP) specification* — transcribed for this
crate in [`docs/rtmp.md`](docs/rtmp.md). (`docs/rtmp.md` uses its own §-numbering
for navigation, not the Adobe spec's own §5.x/§7.x numbering cited throughout
this README and the source doc comments.)

## Features

| Feature | Default | Adds |
|---|---|---|
| `serde` | off | `Serialize`/`Deserialize` on the owned public wire/event types: `ServerEvent`, `ServerConfig`, `Amf0Value`, `Command`. |
| `tokio` | off | The real-socket adapter (`io::AsyncRtmpServer` / `RtmpConnection`); the sans-IO core itself needs no runtime. |

## MSRV

Rust **1.95.0**.

## License

MIT OR Apache-2.0.
