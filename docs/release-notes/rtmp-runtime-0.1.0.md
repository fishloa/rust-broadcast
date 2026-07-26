# rtmp-runtime 0.1.0

Initial release. A sans-IO **RTMP 1.0 ingest** (publish) session engine — the
server side of Adobe's Real-Time Messaging Protocol, for receiving a live push
from an encoder or OBS.

## Why

Every encoder and OBS speaks RTMP push. This crate provides the RTMP wire layer
and session state machine — handshake, chunk-stream (de)assembly, AMF0 command
routing, and the publish flow — as a **bytes-in → (bytes-out, events)** engine
with no I/O of its own, mirroring `rtsp-runtime`. It hands media off as **FLV
bytes** for a container demuxer (`transmux`) to turn into samples; it is
**ingest-only** (publish), never en/decoding media.

## What's in it

- **Handshake** (simple; interoperates with ffmpeg/OBS `-f flv`), **chunk stream**
  (basic + message headers fmt 0-3, extended timestamp, incremental
  `ChunkAssembler` + `ChunkWriter` with `SetChunkSize` tracking), **protocol/user
  control** messages, **AMF0** value + `Command` codec, and the **publish session**
  (`connect`/`createStream`/`publish` + control replies + FLV emission +
  `Acknowledgement` accounting + optional `expected_stream_key` gating).
- `ServerSession::handle_data(&[u8]) -> (Vec<u8>, Vec<ServerEvent>)`; events:
  `Connected` / `Publish` / `Media{flv}` / `Eof`.
- Optional **`tokio`** feature: `io::AsyncRtmpServer` listener + `RtmpConnection`.
- Every wire type has symmetric `Parse`/`Serialize` + byte-identical round-trips,
  verified against a real ffmpeg RTMP publish capture, and the full publish flow
  is decoded end-to-end into a `transmux::Media` (H.264 + AAC) in an integration
  test.

## Spec grounding

Adobe RTMP 1.0, transcribed in `rtmp-runtime/docs/rtmp.md`. AMF3 / egress / `play`
are out of scope for this release.

## Consuming it

Pair with `transmux::StreamingFlvDemux` (new in transmux 0.19) to turn the
incremental `ServerEvent::Media` FLV stream into samples with bounded memory —
that is how `multimux`'s new `InputSpec::Rtmp` push-ingest input is built.
