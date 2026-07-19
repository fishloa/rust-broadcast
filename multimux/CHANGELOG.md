# Changelog

## [Unreleased]

### Added
- **Supervised route lifecycle** (issue #663, P0.2+P0.3+P0.4): each route's
  ingest task is now driven by `origin::supervisor::supervise`, which
  reconnects with capped exponential backoff (`origin::supervisor::Backoff`,
  default 500ms min / 30s max / factor 2.0) on connect failure, pipeline
  error, *or* source end-of-stream, instead of the old one-shot task that
  died for good on the first failure (leaving the HTTP origin serving a
  frozen last playlist as `200 OK` forever). The connect step is abstracted
  behind `origin::supervisor::SourceConnector` (implemented for
  `source::rtsp::RtspSource`) so reconnect is testable without a real RTSP
  server. A route never gives up permanently by default — sources like
  cameras come back.
- **Store health** (`store::MediaStore::{health, set_health}` /
  `store::HealthState`): each route's `MediaStore` now tracks
  `Connecting`/`Live`/`Reconnecting`/`Failed`, set by the supervisor at each
  connect/pipeline transition; a state change bumps the store's existing
  progress watch so a blocked reader (e.g. an LL-HLS long-poll reload) wakes
  on a health transition too, not just new media.
- **Graceful shutdown**: `origin::serve` now installs a shutdown signal
  (Ctrl-C, plus `SIGTERM` on unix) that both drains in-flight HTTP requests
  via `axum::serve(..).with_graceful_shutdown(..)` and breaks every route's
  supervise loop; `serve` joins each supervisor task (aborting one that
  doesn't return within a short grace period) before returning `Ok(())`,
  rather than leaving ingest tasks running detached past shutdown.
- **Structured errors + secret redaction + tracing** (issue #663, P1a):
  - `error::MultimuxError` replaces the stringly `Config(String)`/
    `Source(String)` variants with field-carrying `thiserror` variants
    (`ConfigRead`/`ConfigParse`/`ConfigInvalid`/`Connect`/`Protocol`/`Sdp`/
    `Auth`/`Depay`, plus the existing `Transmux`/`Io`), so callers can match
    on failure *kind* instead of parsing a string, following the
    `rtsp-runtime` convention.
  - **Secret redaction**: an RTSP source URL's `user:pass@` userinfo can no
    longer leak into an error message, `Debug` output, or a log line.
    `config::Route` and `source::rtsp::RtspSource` now have manual `Debug`
    impls that redact `rtsp_url`/`url` to `***@host`; every connect-time
    error path (bad URL parse, connect/TLS/SNI failure, userinfo-stripping
    failure) redacts or uses the already userinfo-stripped URL rather than
    the raw credentialed one.
  - `tracing` throughout: `origin::supervisor::supervise` is
    `#[tracing::instrument]`ed per route (connect/live `info!`, disconnect/
    reconnect `warn!` with backoff delay + attempt count, health
    transitions logged) and `origin::serve` logs startup/shutdown/aborted
    supervisor tasks, replacing the ingest supervisor's `eprintln!`s. The
    library only emits events — `multimux-cli` owns subscriber init
    (`tracing-subscriber` `fmt` + `EnvFilter`, `RUST_LOG`-overridable,
    default `info`); the CLI's own top-level fatal-error report stays a
    plain `eprintln!` so it's never swallowed by a log filter.
- **Prometheus metrics + health/readiness endpoints** (issue #663, P1c):
  - New `prometheus` module: installs a single process-wide
    `metrics-exporter-prometheus` recorder (idempotent — safe to call from
    every `AppState::new`, including many tests sharing one process) and
    renders its snapshot for `GET /metrics`.
  - Metrics recorded throughout the crate via the `metrics` facade:
    `multimux_route_up` (gauge, `route`; mirrors `HealthState` — 1.0 while
    `Live`), `multimux_source_reconnects_total` (counter, `route`; bumped by
    `origin::supervisor::supervise` on each `Reconnecting` transition),
    `multimux_segments_produced_total`/`multimux_parts_produced_total`
    (counters, `route`; bumped in `pipeline::run_pipeline`, which now takes a
    `route: &str` parameter for this label),
    `multimux_active_blocking_requests` (gauge; inc/dec around
    `output::llhls`'s blocking `wait_for_progress`/`wait_for_part` waits via
    an RAII guard), and `multimux_http_requests_total`/
    `multimux_http_request_duration_seconds`/`multimux_bytes_served_total`
    (labels `route`, `path`, and `status` for the requests counter; recorded
    by a new `origin::router` global middleware layer for every request,
    root endpoints included). Cardinality is bounded on purpose: `route` is a
    configured stream name or `"unknown"`, `path` is one of a small fixed set
    of kinds (`playlist`/`segment`/`part`/`init`/`metrics`/`health`/`other`).
  - `GET /healthz` (liveness, always 200) and `GET /readyz` (readiness: 200
    once at least one configured route is `Live`, 503 otherwise) mounted at
    the origin root alongside `/metrics`, above the per-stream `/{stream}/`
    nests.
  - `origin::AppState` gained a `metrics_handle` field and an `AppState::new`
    constructor (replacing the old bare struct literal at every call site).

### Fixed
- **LL-HLS spec-conformance** (issue #663, P2 — RFC 8216bis):
  - `#EXT-X-TARGETDURATION` is now `round(max(configured target, max actual
    segment duration ever seen))`, not `ceil(configured target)`. The
    segmenter cuts on the next keyframe *after* the configured target, so a
    real segment routinely runs longer than the configured value — advertising
    the configured target alone under-declared TARGETDURATION and violated RFC
    8216bis §4.4.3.1 (a MUST: every Media Segment's rounded EXTINF ≤
    TARGETDURATION). `store::MediaStore` now tracks a lifetime
    `max_segment_duration` (never reset on window eviction) that the LL-HLS
    renderer folds into the tag.
  - Blocking-reload `_HLS_msn` semantics (§6.2.5.2): a bare `_HLS_msn` (no
    `_HLS_part`) now waits until segment `msn` is a fully-present **closed**
    Media Segment, rather than resolving as soon as the segment merely *opens*
    with one live part (the old `unwrap_or(0)` conflated it with
    `_HLS_part=0`). `_HLS_msn`+`_HLS_part` keeps the existing part-count
    semantics.
  - `_HLS_msn`/`_HLS_part` abuse bounds (§6.2.5.2): `_HLS_part` without
    `_HLS_msn`, or an `_HLS_msn` more than a small bound beyond the current
    live edge, is now rejected promptly with `400 Bad Request` instead of
    always blocking to the 5 s timeout and returning `200`.
  - `Cache-Control` + permissive CORS on every origin response: immutable
    `max-age=31536000, immutable` on init/segment/part byte ranges, `no-cache`
    on playlists, and `Access-Control-Allow-Origin: *` (+ methods/headers, with
    an `OPTIONS` preflight handler) on everything — browser LL-HLS players
    (hls.js) are commonly on a different origin than the API.

## [0.2.2] - 2026-07-18

### Fixed
- **LL-HLS preload-hint parts no longer 404 at every segment boundary.** The
  segmenter emits a segment's *final* part and closes the segment in the same
  step; `add_segment` then evicted that segment's parts from `live_parts`
  immediately — so the final part (exactly the one the `#EXT-X-PRELOAD-HINT`
  points at) existed for only microseconds, and the in-flight blocking part
  request that raced the close still 404'd. 0.2.1 made not-yet-produced parts
  *block*; this makes the just-produced final part *survive*: `add_segment` now
  moves a closed segment's parts into a bounded `recent_parts` buffer that
  `part_bytes` also checks, so the hinted final part is served (HTTP 200) after
  its segment closes instead of 404ing. Eliminates the per-segment 404 spam +
  the boundary latency bump. Bounded oldest-first like `live_parts`; closed
  parts are still not rendered in the playlist (the whole segment is).

### Fixed
- **LL-HLS preload-hint parts no longer 404.** A request for a Partial Segment
  the media playlist promised via `#EXT-X-PRELOAD-HINT` but that the origin had
  not produced yet returned `404` immediately, instead of holding the request
  open until the part became available (RFC 8216bis §6.2.2 / §6.3.1 blocking
  Partial-Segment delivery). Every low-latency client (hls.js, Safari native)
  therefore hammered the hinted part with 404s until it happened to exist,
  spamming errors and forcing a fall back to full-segment loads — defeating the
  low-latency path. The part byte handler now blocks (reusing the same progress
  watch as the blocking playlist reload) until the part is produced, or returns
  `404` *promptly* once its segment closes without it (a real segment boundary)
  or the blocking timeout elapses. Observed against a live on-camera stream.

### Breaking
- The bundled `multimux` **binary** (the RTSP→LL-HLS CLI) moved to a new
  dedicated crate, **`multimux-cli`**. `multimux` is now a **library only**
  (its `serve`/`config`/`origin`/`pipeline`/`source`/`store` API is unchanged).
  `cargo install multimux-cli` provides the `multimux` binary as before. The
  `cli` cargo feature (and the `clap` dependency) were removed from `multimux`.

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
