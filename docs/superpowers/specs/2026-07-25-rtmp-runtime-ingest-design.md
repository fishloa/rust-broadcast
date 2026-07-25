# rtmp-runtime — sans-IO RTMP 1.0 ingest session engine (#738)

> Design spec for #738. Status: authored 2026-07-25 during the autonomous backlog run, from the scouting digest + `rtmp-runtime/docs/rtmp.md` (#762). New independent crate; mirrors `rtsp-runtime`. Ingest-only.

## Problem / scope
Every encoder + OBS speaks **RTMP push** (publish). multimux has no RTMP ingest. Add a new **`rtmp-runtime`** crate — a sans-IO RTMP 1.0 **server session** engine (handshake → chunk (de)assembly → AMF command routing → publish state machine) — plus a `tokio`-gated TCP listener adapter, and wire it into multimux as a first-class `InputSpec::Rtmp` source. transmux keeps the **FLV-body → `Media`** demux (`FlvDemux`); rtmp-runtime hands it reassembled FLV tag bytes.

**In scope:** server-side publish ingest (encoder → us). **Out of scope (non-goals):** RTMP egress / publish-out, `play`/playback (client-pull), AMF3, SharedObject, aggregate-message authoring. A `client` session may be stubbed for symmetry but is NOT required by #738 (unlike rtsp-runtime which needs both; RTMP ingest is server-only).

## Architecture — sans-IO core + optional tokio listener (mirrors rtsp-runtime)
A **bytes-in → (bytes-out, events)** engine, no IO in the core:
- `ServerSession::handle_data(&mut self, input: &[u8]) -> Result<(Vec<u8>, Vec<ServerEvent>), RtmpError>` — feed inbound TCP bytes, get (bytes to write back, typed events). Drives the whole handshake + chunk reassembly + AMF exchange internally; buffers partial chunks across calls.
- No `poll()`; outbound bytes are returned from `handle_data` (rtsp-runtime's exact contract).
- `#[cfg(feature = "tokio")]` `io` module: `AsyncRtmpServer` — binds `:1935`, accepts connections, drives one `ServerSession` per connection, surfaces `ServerEvent`s (the piece multimux consumes).

The sans-IO core needs **no tokio**. Not `#![no_std]` (uses `Vec`), but `#![forbid(unsafe_code)]`. `edition = 2024`, `rust-version = 1.86`, `license = "MIT OR Apache-2.0"`.

### Module layout (mirror rtsp-runtime)
- `handshake` — C0/S0, C1/S1, C2/S2 wire structs (Parse/Serialize) + the handshake sub-FSM.
- `chunk` — basic header (1/2/3-byte csid) + message header fmt 0/1/2/3 + extended timestamp; **`ChunkAssembler`** (inbound: chunks → complete `Message`s, honoring `SetChunkSize`) and **`ChunkWriter`** (outbound: `Message` → chunks). Symmetric Parse/Serialize on the header types.
- `message` — `Message { header, payload }`, message-type-id consts, protocol-control messages (SetChunkSize/Abort/Ack/WindowAckSize/SetPeerBandwidth) + User Control events, all typed (Parse/Serialize + round-trip).
- `amf0` — `Amf0Value` (Number/Boolean/String/Object/Null/EcmaArray/StrictArray/…​ enough for the command set) with Parse/Serialize; `Command { name, transaction_id, arguments }`.
- `server` — `ServerSession` (the FSM), `ServerEvent`, `ServerConfig`.
- `io` `#[cfg(feature="tokio")]` — `AsyncRtmpServer` listener.
- `error` — `RtmpError` (thiserror).

**Reuse note:** transmux already has RTMP wire types (`Message`, `BasicHeader`, `AmfValue`, `ProtocolControl`, `Handshake*`, consts) in `transmux/src/rtmp.rs`, but they are tuned for a **static-buffer skim** (`read_chunks`). rtmp-runtime needs **live, incremental, stateful** chunk assembly across TCP reads with `SetChunkSize` tracking + outbound authoring — a different job. rtmp-runtime owns its own codec (per CRATE-ACCEPTANCE: symmetric Parse/Serialize + byte-identical round-trip); it does NOT depend on transmux's internal rtmp module. The seam to transmux is **FLV bytes**, not RTMP types (§ boundary).

## Server session state machine
```
Init → HandshakeDone → Connected(app) → Publishing(stream_key) → [Media…] → Done
```
- **Handshake**: receive C0+C1 → reply S0+S1+S2 → receive C2 → HandshakeDone. (Use the simple/original handshake; the digest + doc §2 cover the 1536-byte packets. Complex/HMAC handshake not required for ingest interop with ffmpeg/OBS in simple mode; document the choice.)
- **`connect`** (AMF, NetConnection): capture `app` from the command object; reply WindowAckSize + SetPeerBandwidth + SetChunkSize (outbound) + `_result` (NetConnection.Connect.Success). Emit `ServerEvent::Connected { app }`.
- Tolerate OBS extras **`releaseStream` / `FCPublish`** (respond benignly / `_result` where expected; never error).
- **`createStream`** → allocate a message stream id, reply `_result` with it.
- **`publish`** (NetStream): capture `stream_key` (publishing name) + type ("live"). Reply User Control **StreamBegin** + `onStatus` (NetStream.Publish.Start). Transition to Publishing. Emit `ServerEvent::Publish { app, stream_key, stream_id }`.
- **Audio(8)/Video(9)/Data-AMF0(18)** messages while Publishing → convert each to an **FLV tag** (tag type from msg type id, timestamp from the message header, body = payload) and emit `ServerEvent::Media { flv: Vec<u8> }` (a concatenatable FLV tag byte run, incl. the FLV tag header + prevTagSize, ready to feed `FlvDemux`). The very first emission includes the FLV file header (`FLV\x01` + flags + header size + first prevTagSize=0) so the byte run is a valid FLV stream.
- **`deleteStream` / connection close** → `ServerEvent::Eof`.
- **Ack accounting**: track inbound byte count vs the peer's WindowAckSize; emit an `Acknowledgement` when due. **Ack sequence number wraps as plain modular u32** (RTMP spec is silent — documented choice, not RFC1982). **Timestamps** are 32-bit and use **RFC1982 serial arithmetic** on compare/order (doc §1 mandates this for timestamps).

### Events
```rust
#[non_exhaustive]
pub enum ServerEvent {
    Connected { app: String },
    Publish { app: String, stream_key: String, stream_id: u32 },
    Media { flv: Vec<u8> },   // FLV bytes ready for transmux::FlvDemux
    Eof,
}
```
`ServerConfig { chunk_size: u32, window_ack_size: u32, expected_stream_key: Option<String> }` — if `expected_stream_key` is set and the publish key mismatches, reply `onStatus` NetStream.Publish.Failed + emit no `Publish`/`Media` (auth at the session edge; multimux maps its own auth on top).

## Boundary to transmux
rtmp-runtime emits **FLV bytes** (`ServerEvent::Media { flv }`); the consumer feeds them to **`transmux::FlvDemux`** (`Unpackage<Input=&[u8], Media=Media>`) to get the neutral `Media` IR. rtmp-runtime does **not** depend on transmux (keeps the wire crate lean + no dep cycle — multimux owns the FlvDemux call). transmux's existing `RtmpDemux::read_chunks` static path is untouched (still valid for demuxing a captured `.flv`/RTMP buffer offline).

## multimux integration (first-class source)
- `multimux/src/config.rs`: add `InputSpec::Rtmp { listen: String /* "0.0.0.0:1935" */, app: Option<String>, stream_key: Option<String> }` (serde `snake_case`, the `#[non_exhaustive]` enum).
- `multimux/src/source/rtmp.rs`: `RtmpSource` — a **listener-shaped** connector: binds the `AsyncRtmpServer`, accepts a publisher, pumps `ServerEvent::Media` → `FlvDemux` → `Media` → the `MediaStore` (same sink the pull sources feed). Unlike the outward-reconnect pull sources, this **accepts inbound** connections; the supervisor reconnect/`Backoff` shape becomes "re-listen / await next publisher".
- `multimux/src/origin/mod.rs`: add the `InputSpec::Rtmp { .. } => …` match arm building `RtmpSource` + spawning it under the existing supervise/shutdown machinery.
- `multimux` depends on `rtmp-runtime` (+ existing `transmux`).

## Real-fixture gate (CRATE-ACCEPTANCE hard bar)
Generate a **real RTMP publish capture**: run a tiny capturing TCP listener on `:1935` and `ffmpeg -re -i fixtures/ts/h264_aac.ts -c copy -f flv rtmp://127.0.0.1:1935/live/testkey`, recording the exact inbound bytes (handshake C0/C1/C2 + connect/createStream/publish AMF + the first Audio/Video messages) to `rtmp-runtime/tests/fixtures/obs-publish.bin` (+ a PROVENANCE note with the exact ffmpeg command). Biting tests:
- `ServerSession::handle_data` fed the captured stream reaches `Publishing`, emits `Connected{app:"live"}` + `Publish{stream_key:"testkey"}` + ≥1 `Media`, and the concatenated `Media.flv` feeds `transmux::FlvDemux` (as a dev-dependency in the test) to yield a `Media` with the expected track kinds.
- **Byte-identical round-trip** on the chunk/message/AMF0/handshake wire types (parse → serialize == input) over the fixture's frames — the hard invariant.
- Mutation-checks: a neutered chunk assembler / AMF parser fails these.

## Non-goals
Egress/publish-out, `play`/playback, AMF3, SharedObject, HMAC/complex handshake (simple handshake only), authenticated-CDN semantics. A future issue can add the client/egress side.

## Release
New crate → clears `docs/CRATE-ACCEPTANCE.md` (round-trip, no raw-byte API, real-fixture bite, 6-gate, #204 label convention on every public enum, fuzz target, ≥2 examples, full RELEASE-DOCS). New `release-rtmp-runtime.yml` lane (copy `release-rtsp-runtime.yml`; tag pattern **`rtmp-runtime-v*`**; publishes after transmux, before multimux consumes it). Independent version; first release `0.1.0`. Tag/publish awaits explicit owner sign-off.
