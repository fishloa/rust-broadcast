# rtmp-runtime

[![crates.io](https://img.shields.io/crates/v/rtmp-runtime.svg)](https://crates.io/crates/rtmp-runtime)
[![docs.rs](https://img.shields.io/docsrs/rtmp-runtime)](https://docs.rs/rtmp-runtime)

Sans-IO RTMP 1.0 publish engine, both ends: an ingest server that receives a live push from OBS or ffmpeg, and a publish client that pushes to a remote RTMP server. An optional `tokio` adapter provides sockets.

## What it does / what it does not

**Does:** own the RTMP wire layer and the publish session. That is the handshake, chunk-stream assembly and writing, protocol control messages, AMF0 commands, and the `connect`, `createStream`, `publish` flow. The server hands the received audio, video and data back as **FLV bytes** (feed them to `transmux`'s FLV demuxer); the client takes already-encoded audio, video and metadata to send.

**Does not:** play or pull a stream (egress is out of scope on both sides), encode or decode media, run the complex (HMAC) handshake, support RTMPE/RTMPS, or implement AMF3 beyond decoding AMF3-encoded command messages. There are no sockets in the core: you feed bytes in and write the returned bytes out.

Typical users: a live-ingest origin (`multimux`'s RTMP input and push output), test publishers, capture tooling.

## Standards supported

| Standard | Edition | Scope | Status | Notes |
|---|---|---|---|---|
| Adobe RTMP specification 1.0 | 1.0 (vendored `adobe_rtmp_1.0`, transcribed in `docs/rtmp.md`) | §5.2 handshake (simple), §5.3 chunk stream (fmt 0 to 3, 1/2/3-byte csid, extended timestamp), §5.4 protocol control, §6 message format, §7.1 message types, §7.2 command messages for the publish flow | Partial | Publish only; no play, no complex handshake. `docs/rtmp.md` uses its own section numbering |
| Adobe AMF0 (referenced as `[AMF0]` by the RTMP spec) | Not separately vendored | Value types and the command set used for ingest | Partial | AMF3 commands (type 17) decode like AMF0; Data-AMF3 (15) and Shared-Object (16/19) are out of scope |
| Adobe FLV (file format v10.1) | v10.1 (vendored `adobe_flv_f4v_v10_1`) | Server emits FLV tags for audio, video and script data | Full for output | Demux with `transmux` |

Verified against a real ffmpeg publish capture (`tests/fixtures/obs-publish.bin`, provenance in `tests/fixtures/PROVENANCE.md`), replayed whole and in small chunks, and decoded by `transmux` into an H.264 plus AAC `Media`.

## Features

| Feature | Default | Description |
|---|---|---|
| `serde` | no | `Serialize`/`Deserialize` on `ServerEvent`, `ServerConfig`, `Amf0Value`, `Command`. |
| `tokio` | no | `io::AsyncRtmpServer`, `RtmpConnection`, `io::AsyncRtmpClient`, `io::RtmpTimeouts` and `target::RtmpTarget` (`rtmp://host[:port]/app/key`, parsed with `url`). |

The core is `std` and needs no runtime; it has `#![forbid(unsafe_code)]`.

## Quick start

Replay the real ffmpeg capture through the server session:

```rust
use rtmp_runtime::server::{ServerEvent, ServerSession};

let capture = std::fs::read("tests/fixtures/obs-publish.bin").unwrap();
let mut session = ServerSession::with_defaults();
let mut flv = Vec::new();
let mut published = false;
for chunk in capture.chunks(1400) {
    let (_reply, events) = session.handle_data(chunk).unwrap();
    for ev in events {
        match ev {
            ServerEvent::Connected { app } => assert_eq!(app, "live"),
            ServerEvent::Publish { stream_key, .. } => {
                assert_eq!(stream_key, "testkey");
                published = true;
            }
            ServerEvent::Media { flv: bytes } => flv.extend_from_slice(&bytes),
            _ => {}
        }
    }
}
assert!(published && flv.starts_with(b"FLV"));
```

Or connect the two roles in memory (a publisher and an ingest server, no socket):

```rust
use rtmp_runtime::client::{ClientConfig, ClientEvent, ClientSession};
use rtmp_runtime::server::{ServerConfig, ServerEvent, ServerSession};

let mut config = ClientConfig::default();
config.app = "live".into();
config.stream_key = "key".into();
let mut client = ClientSession::new(config);
let mut server = ServerSession::new(ServerConfig::default());

let mut to_server = client.start();
let (mut publishing, mut accepted) = (false, false);
for _ in 0..32 {
    let (to_client, sevents) = server.handle_data(&to_server).unwrap();
    accepted |= sevents.iter().any(|e| matches!(e, ServerEvent::Publish { .. }));
    let (next, cevents) = client.handle_data(&to_client).unwrap();
    publishing |= cevents.iter().any(|e| matches!(e, ClientEvent::Publishing));
    to_server = next;
    if publishing && accepted && to_server.is_empty() { break; }
}
assert!(client.is_publishing() && accepted);
```

With the `tokio` feature, `io::AsyncRtmpServer` binds a listener and drives one `ServerSession` per connection, and `io::AsyncRtmpClient` publishes to an `RtmpTarget` URL. See the `ingest_to_flv` and `capture_publish` examples.

## What is implemented

- **Handshake** (`handshake`): C0/S0, C1/S1, C2/S2, simple form (interoperates with ffmpeg and OBS).
- **Chunk stream** (`chunk`): all four `fmt` header forms, extended timestamps, incremental `ChunkAssembler` (header inheritance, `SetChunkSize`, partial input), `ChunkWriter`.
- **Messages** (`message`): protocol control (SetChunkSize, Abort, Acknowledgement, WindowAckSize, SetPeerBandwidth), User Control events, the message type catalogue.
- **AMF0** (`amf0`): `Amf0Value` and `Command` encode and decode.
- **`server::ServerSession`**: `connect`, `createStream`, `publish`, plus the OBS extras `releaseStream` and `FCPublish`; replies `_result`, StreamBegin and `onStatus`; acknowledgement accounting; optional `expected_stream_key` gate. Events: `Connected`, `Publish`, `Media`, `Eof`, `Unsupported`.
- **`client::ClientSession`**: client handshake and the auto-advancing publish flow, then `send_audio`, `send_video`, `send_metadata`.
- **Security defaults:** `ServerConfig::expected_stream_key = None` admits any publisher, so a server reachable from untrusted networks must set a key (compared in constant time). `Debug` never prints the stream key. In-progress message size is capped by `max_in_progress_bytes`.

## Design notes

Bytes-in, (bytes-out, events) engine with partial-input buffering, so it runs on any transport. Wire structures implement symmetric `Parse`/`Serialize`; a chunk `message length` over 24 bits is rejected (`FieldOverflow`) instead of truncated. `RtmpError` is `#[non_exhaustive]`.

## Related crates

Depends on `broadcast-common`; `transmux` (a dev-dependency here) demuxes the FLV the server emits. `multimux` builds its RTMP input and push on this crate; `rtsp-runtime` is the sibling sans-IO client and server engine.

## MSRV and licence

MSRV 1.95.0. Licensed under MIT OR Apache-2.0.
