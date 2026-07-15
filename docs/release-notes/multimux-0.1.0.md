# multimux 0.1.0 — 2026-07-15

First release. A **live RTSP → LL-HLS just-in-time repackaging HTTP origin**
(issue #663) — a thin client + server wrap around two existing workspace
crates: `rtsp-runtime` (RTSP 1.0 pull) and `transmux` (RTP depayload + LL-HLS
CMAF segmentation). Pulls one or more live RTSP sources and serves each as
Low-Latency HLS (RFC 8216bis) from an in-process tokio + axum HTTP origin.
Muxing only — samples stay opaque, never transcoded.

## Added (#663)

- **`config`** — `Config`/`Route`: CLI-first with an optional JSON config file
  for multiple routes (`bind`, `target_duration_secs`, `part_target_ms`,
  `window_segments`, `routes: [{ name, rtsp_url }]`); `Config::validate()`
  rejects empty route sets, duplicate stream names, and non-positive
  timing/window values.
- **`source::rtsp`** — `RtspSource`/`RtspSession`: DESCRIBE → SETUP
  (interleaved TCP, per-media channel pair) → PLAY over
  `rtsp_runtime::io::AsyncRtspClient`; per-track `CodecConfig` from the
  DESCRIBE SDP via `transmux`'s SDP-fmtp → codec-config helpers; interleaved
  RTP routed per channel into `transmux::RtpStreamDepacketizer`, yielding
  timed `Sample`s ready for segmentation.
- **`pipeline`** — `run_pipeline` drives a `SampleSource` (the real
  `RtspSession`, or `MockSource` for tests/examples) through a
  `transmux::ll_hls::LlHlsSegmenter` into a `StreamStore`, publishing every
  init segment / ready part / ready segment, and flushing the buffered tail
  at end-of-stream so no trailing samples are silently dropped.
- **`store::StreamStore`** — per-stream in-RAM rolling window: the init
  segment, a bounded `VecDeque` of closed full segments (oldest evicted on
  roll), and the in-progress segment's live parts; a `tokio::sync::watch`
  bumped on every new part/segment. Renders the LL-HLS media playlist per
  RFC 8216bis — `#EXT-X-PART-INF`/`#EXT-X-SERVER-CONTROL`/`#EXT-X-PART`/
  `#EXT-X-PRELOAD-HINT` — and never advertises an `#EXTINF`/URI for a segment
  that hasn't closed yet (§4.4.4.9).
- **`origin`** — the axum HTTP surface: `router` (`master.m3u8`, `media.m3u8`
  with `_HLS_msn`/`_HLS_part` blocking reload per §6.2.5.2 bounded to a fixed
  timeout, and a catch-all for the dynamic `init-*.mp4`/`seg-*.m4s`/
  `part-*.m4s` filenames the playlist emits) and `serve(config)` — the full
  entrypoint: one `StreamStore` + one spawned per-route RTSP pipeline task per
  configured route, then bind + serve. A route whose source fails to connect
  or whose pipeline errors logs to stderr and ends only that task; one bad
  camera never takes down the server or any other route.
- **CLI** (`multimux` binary, `cli` feature, on by default) — `--config
  <FILE>` (JSON, multi-route) or the single-route quick start `--rtsp <URL>
  --name <NAME>`, plus `--bind`/`--target-duration`/`--part-ms`/`--window`,
  following `docs/CLI-STANDARD.md` (clap derive, named flags, auto
  `--help`/`--version`).
- **Examples** — `serve_mock` (a synthetic stream through the real
  segmenter + origin, no RTSP source or network dependency needed to try it)
  and `serve_rtsp` (serves one real RTSP URL given on the command line).

## Correctness

`multimux/tests/origin_llhls.rs` drives the whole pipeline-to-HTTP path with a
deterministic `MockSource` (no real network/timing dependency): the served
init segment and a served media segment are validated with `transmux`'s fMP4/
CMAF conformance validator (zero `Severity::Error` issues), the media
playlist is asserted to carry the real LL-HLS tags, and a second test proves
the blocking-reload path genuinely wakes on the `watch` channel as soon as a
part lands — bounded in *real* time well under the handler's internal
timeout fallback, so a broken wakeup (falling through to the 5 s timeout)
would fail the test rather than pass slowly.

## v1 scope and limits (documented, by design)

- **LL-HLS only** — DASH/LL-DASH is deferred to v1.1; `transmux`'s DASH
  packager already exists, multimux just doesn't wire it up yet.
- **RTSP pull only** — no SRT/TS/file ingest.
- **No per-viewer sessions, SSAI, or manifest rewrites.**
- **No DVR/VOD/disk spill** — the window is RAM-only and rolls forward; once
  a segment is evicted it's gone.
- **No TLS/auth** — run a reverse proxy in front for either.
- **No trick-play.**
- Inherited from `transmux`'s streaming depayloader (issue #700, the
  upstream prerequisite this release consumes): low-delay H.264 only (no
  B-frame DTS reconstruction), one AAC access unit per RTP packet, and
  packets must be fed in arrival order — all satisfied by a direct RTSP pull
  over interleaved TCP.

## Compatibility

First release — no prior API to break. Depends on `transmux` 0.17 (the
streaming RTP depayloader + SDP-fmtp helpers landed there, issue #700) and
`rtsp-runtime` 0.2 (`tokio` feature).
