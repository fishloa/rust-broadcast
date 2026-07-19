# Changelog

All notable changes to `ll-hls-client` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **`LlHlsClient` — sans-IO Low-Latency HLS playback client engine** (issue
  #717, slices 2-4). A caller-driven state machine in the same sans-IO shape
  as `srt-runtime` (#565): `poll()`/`next_output()` drain queued `Action`s /
  `Output`s; `on_playlist()`/`on_resource()`/`on_error()` feed responses back
  in. No socket, no clock, no `tokio`/`reqwest` dependency in the core.
  - **Reload scheduler** (slice 2): Blocking Playlist Reload
    (`_HLS_msn`/`_HLS_part`, RFC 8216bis §6.2.5.2) once a playlist advertises
    Low-Latency support, correctly distinguishing a bare `_HLS_msn` (waits for
    a closed segment) from `_HLS_part=0`; non-blocking-reload backoff derived
    from `#EXT-X-TARGETDURATION` for non-LL origins; best-effort `EXT-X-SKIP`/
    `CAN-SKIP-UNTIL` Playlist Delta Update merge.
  - **Fetch pipeline** (slice 3): `EXT-X-PRELOAD-HINT` part prefetch ahead of
    its own numbered appearance; `BYTERANGE` part/segment/map support
    (including the "omitted offset continues the previous sub-range" rule);
    the init segment (`EXT-X-MAP`) fetched once.
  - **Output adapter** (slice 4): ordered `Output::Init` then `Output::Samples`
    (real access units via `transmux::Fmp4Demux`, not opaque container bytes);
    `EXT-X-DISCONTINUITY` forwarded as `Output::Discontinuity`; parts already
    individually fetched are never double-counted when their parent segment
    later closes (dedup/coalescing); a non-LL playlist (no parts at all) plays
    via the full-segment fallback path; resources arriving before the init
    segment are buffered and replayed once it arrives.
  - Reuses `transmux::hls::MediaPlaylist::parse` (issue #717 slice 1) for the
    playlist model — this crate defines no playlist types of its own.
  - `tests/origin_loop.rs`: an in-process origin↔client loop against a real
    `transmux::ll_hls::LlHlsSegmenter`, asserting the exact blocking-reload
    `_HLS_msn`/`_HLS_part` requested, the preload-hint prefetch actually
    issued, ordered/deduped/byte-identical sample reconstruction, and the
    non-LL full-segment fallback path.
  - `CAN-BLOCK-RELOAD` (issue #717 slice 1 follow-up, fixed alongside slice
    5): reload scheduling now honours `transmux::hls::LowLatencyConfig::can_block_reload`
    rather than inferring blocking-reload support from `low_latency` being
    `Some` — an origin advertising `CAN-BLOCK-RELOAD=NO` (while still
    carrying `PART-INF`/`PART` tags) now correctly gets a plain, non-blocking
    reload paced by `Action::WaitMs`, never a blocking `_HLS_msn`/`_HLS_part`
    request. Covered by `tests/origin_loop.rs`'s
    `can_block_reload_no_yields_non_blocking_reload_with_backoff`.
- **`TokioClient` — tokio + reqwest (rustls) IO adapter** (issue #717 slice
  5), behind a new, non-default `tokio` cargo feature. Drives `LlHlsClient`
  over real HTTP: performs the blocking `_HLS_msn`/`_HLS_part` reload and
  plain playlist GETs, resource fetches (including `Range` byte-ranges),
  retries resource fetches with capped backoff before falling back to
  `on_error` (letting the next reload naturally re-request them), and
  retries a failing playlist reload indefinitely with capped backoff (the
  sans-IO core has no other recovery path for a playlist fetch failure).
  Optional HTTP Basic/Bearer auth via `TokioClientConfig::auth`, with a
  documented TODO to swap in the workspace's planned shared multi-scheme
  auth crate once it exists. Exposes `TokioClientStats` (playlist fetches,
  blocking reloads, resource fetches, preload-hint-triggered resource
  fetches) so blocking-reload/prefetch behaviour is externally observable,
  not just internally exercised. The sans-IO core (`client.rs`) gained no
  new dependency from this — `tokio`/`reqwest` are entirely behind the new
  feature.
  - `tests/glass_to_glass.rs` (gated on the `tokio` feature; epic #717's
    done-bar): drives `TokioClient` against a **real** `multimux`-served
    LL-HLS origin over real loopback HTTP, fed by a real-time-paced
    `transmux::ll_hls::LlHlsSegmenter` producer (live-shaped, ~30fps/120ms
    parts) — measures glass-to-glass latency (wall-clock push-to-emit,
    embedded per-sample) and asserts it is **sub-second**, asserts at least
    one Blocking Playlist Reload and one preload-hint-triggered resource
    fetch actually occurred (`TokioClientStats`), and asserts a genuinely
    non-LL playlist (no `PART` tags, served from a minimal hand-built axum
    origin) still plays via the full-segment fallback with zero blocking
    reloads.
