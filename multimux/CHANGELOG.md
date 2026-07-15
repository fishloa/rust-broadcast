# Changelog

## [Unreleased]

## [0.1.0] - 2026-07-15
### Added
First release (issue #663): a live RTSP → LL-HLS just-in-time repackaging HTTP
origin — a thin client + server wrap around `rtsp-runtime` (RTSP pull) and
`transmux` (RTP depayload + LL-HLS CMAF segmentation).

- **Config** (`config::Config`/`Route`): CLI-first, with an optional JSON
  config file for multiple routes; `bind`, `target_duration_secs`,
  `part_target_ms`, `window_segments`, and `routes: [{ name, rtsp_url }]`;
  `Config::validate()` rejects empty route sets, duplicate stream names, and
  nonsensical timing/window values.
- **RTSP ingest** (`source::rtsp::RtspSource`/`RtspSession`): DESCRIBE → SETUP
  (interleaved TCP, one media per SETUP) → PLAY over
  `rtsp_runtime::io::AsyncRtspClient`; SDP → per-track `CodecConfig` via
  `transmux`'s SDP-fmtp helpers; interleaved RTP routed per channel into
  `transmux::RtpStreamDepacketizer`, yielding timed `Sample`s.
- **Per-route pipeline** (`pipeline::run_pipeline`): drives a `SampleSource`
  (real `RtspSession` or, for tests/examples, `MockSource`) through a
  `transmux::ll_hls::LlHlsSegmenter`, publishing every init segment, ready
  part, and ready segment into a `StreamStore`; flushes the buffered tail at
  end-of-stream.
- **`StreamStore`** (`store::StreamStore`): per-stream in-RAM rolling window
  (init segment + a bounded `VecDeque` of closed segments + the in-progress
  segment's live parts), oldest segment evicted on roll; a
  `tokio::sync::watch` bumped on every new part/segment drives blocking
  playlist reload; renders the LL-HLS media playlist per RFC 8216bis
  (`#EXT-X-PART-INF`/`#EXT-X-SERVER-CONTROL`/`#EXT-X-PART`/
  `#EXT-X-PRELOAD-HINT`), never advertising an `#EXTINF`/URI for an
  unclosed segment (§4.4.4.9).
- **HTTP origin** (`origin::{router, serve}`, axum): `master.m3u8`,
  `media.m3u8` (blocking reload on `_HLS_msn`/`_HLS_part`, RFC 8216bis
  §6.2.5.2, bounded so a stalled source can't hang a request forever), and a
  catch-all serving the dynamic `init-*.mp4`/`seg-*.m4s`/`part-*.m4s`
  filenames the playlist emits. `origin::serve(config)` wires one
  `StreamStore` + one spawned RTSP pipeline task per configured route, then
  binds and serves — a single bad/unreachable source logs to stderr and ends
  only that route's task, never the server.
- **CLI binary** (`multimux`, `cli` feature, on by default): `--config <FILE>`
  or the single-route quick start `--rtsp <URL> --name <NAME>`, plus
  `--bind`/`--target-duration`/`--part-ms`/`--window`, per
  `docs/CLI-STANDARD.md`.
- **Examples**: `serve_mock` (synthetic stream, no RTSP/network needed) and
  `serve_rtsp` (serves one real RTSP URL given on the command line).

### v1 scope
LL-HLS only (DASH/LL-DASH is v1.1); RTSP pull only (no SRT/TS/file ingest); no
per-viewer sessions/SSAI/manifest rewrites; no DVR/VOD/disk spill (RAM-only
rolling window); no TLS/auth (front it with a reverse proxy); no trick-play.
