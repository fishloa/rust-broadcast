# `obs-publish.bin` provenance

- **Captured**: 2026-07-25
- **Source A/V**: `fixtures/ts/h264_aac.ts` (workspace-committed TS fixture; H.264 Main 320x240 25fps + AAC-LC 44.1kHz mono)
- **Capture harness**: `rtmp-runtime/examples/capture_publish.rs` (`--features tokio`) — binds `127.0.0.1:1935`, accepts one connection, drives the real sans-IO `rtmp_runtime::server::ServerSession::handle_data`, writes back the session's reply bytes, and records every raw inbound byte until the peer closes (or an `Eof` event fires), writing the accumulated bytes to this file.
- **Publisher**: `ffmpeg version 8.1.2` (Homebrew, macOS/arm64), real simple RTMP handshake (no HMAC/complex handshake needed — ffmpeg's `-f flv` publisher interoperated on the first attempt).
- **Exact command**:
  ```
  ffmpeg -re -i fixtures/ts/h264_aac.ts -t 2 -c copy -f flv rtmp://127.0.0.1:1935/live/testkey
  ```
- **Result**: real handshake completed, `connect` (`app=live`) → `createStream` → `publish` (`stream_key=testkey`, `stream_id=1`) all succeeded against `ServerSession`; 144 `ServerEvent::Media` events were emitted before ffmpeg closed the stream (`ServerEvent::Eof` observed twice — `FCUnpublish` then `deleteStream`, both routed to the same `Eof` event).
- **Size**: 47.6 KiB raw inbound bytes (handshake + chunk-encoded connect/createStream/publish/A-V messages).
- **Replay**: `rtmp-runtime/tests/ingest_fixture.rs` reads this file via `std::fs`, replays it through a fresh `ServerSession::handle_data` (in one call and in small chunks, to exercise partial-input reassembly), and feeds the concatenated emitted FLV to `transmux::FlvDemux` to confirm it decodes to a real `Media` with an H.264 video track and an AAC audio track.
