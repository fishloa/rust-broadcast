# rtmp-runtime 0.8.1

_Released 2026-11-02._

### Added
- **Play mode for the sans-IO client** (issue #1460): `ClientConfig::mode` (`ClientMode::{Publish, Play}`, default `Publish`) and `ClientConfig::play_buffer_ms` (default `DEFAULT_PLAY_BUFFER_MS`). In play mode `ClientSession` runs `connect` -> `createStream` -> `SetBufferLength` + `play` (Adobe RTMP 1.0 §7.2.2.1, §6.7) with the stream name in `stream_key`, and reports `ClientEvent::Playing` (`NetStream.Play.Start`), `ClientEvent::Media { flv }` (Audio/Video/Data-AMF0 and unpacked Aggregate messages as FLV tags, the first chunk prefixed with the FLV file header, exactly as `ServerEvent::Media`), `ClientEvent::StreamEnded` (Stream EOF or `NetStream.Play.Stop`/`Complete`) and, as `ClientEvent::Error`, `NetStream.Play.StreamNotFound`, `NetStream.Play.Failed` and any `onStatus` of level `error`. Set Chunk Size, Window Acknowledgement Size/Acknowledgement and PingRequest/PingResponse behave as on the publish side; AMF3 command messages are decoded after their marker byte. Publish mode is unchanged.
### Changed
- The FLV tag writer and the Aggregate (22) unpacking moved from `server.rs` into a crate-private `flv_bridge` module shared by the server and the client; no behaviour change.
### Documentation
- Fixed documentation links/citations to resolve to existing paths (docs/tools hygiene).

---

Published from tag `rtmp-runtime-v0.8.1`.
