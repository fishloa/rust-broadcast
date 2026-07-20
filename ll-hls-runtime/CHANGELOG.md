# Changelog

All notable changes to `ll-hls-runtime` are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed
- **`server::master_playlist_m3u8` now takes a `media_playlist_name: &str`
  argument** (issue #663 "shared output auth + configurable playlist_name"):
  the master playlist's `#EXT-X-STREAM-INF` reference is the caller's
  configured media-playlist filename instead of the hardcoded `"media.m3u8"`
  literal, so a server (e.g. multimux's `Config::playlist_name`) can serve
  its media playlist under any `*.m3u8` name. Breaking: pass the intended
  filename explicitly (`master_playlist_m3u8("media.m3u8")` reproduces the
  old behaviour).
- **`client::tokio_client::TokioClient` now authenticates via `broadcast-auth`**
  (issue #663 P3c): `TokioClientConfig::auth` takes a
  `broadcast_auth::Credentials` (Basic/Digest/Bearer) instead of the ad hoc
  `Auth` enum (Basic/Bearer only) — fulfilling the TODO the field's doc
  comment carried since P3a. Basic/Bearer are still pre-applied on every
  request via reqwest's own helpers; Digest now works end-to-end: on a `401`,
  `TokioClient` reads `WWW-Authenticate`, computes the response via
  `broadcast_auth::Authenticator`, resends once, and caches the resulting
  authenticator (applied preemptively, advancing `nc`, on later requests).
  New `TokioError::Auth` variant for a challenge/response failure. Breaking:
  `tokio_client::Auth` is removed; construct `broadcast_auth::Credentials`
  instead (added as a `tokio`-feature-gated optional dependency).

### Added

#### Server (issue #663/#717 Stage 2)

- **`server` — the sans-IO LL-HLS origin engine** (Stage 2 of the
  ll-hls-runtime unification, issue #663/#717 —
  `docs/superpowers/specs/2026-07-18-multimux-hub-design.md`, "ll-hls-runtime
  — client + server in one crate"), moved out of `multimux` behind the new
  `std` feature (needs `std::sync::Mutex`, unlike the no_std-capable
  `client`):
  - **`server::MediaStore`** — the protocol-neutral rolling in-RAM window
    (init/segments/live parts/`recent_parts`/health/max-segment-duration),
    moved verbatim from `multimux::store::MediaStore` **including the
    part-404-boundary fix** (`recent_parts`, so an in-flight preload-hint
    request for a segment's final part still resolves after the segment
    closes). The `tokio::sync::watch<u64>` progress signal is replaced with a
    runtime-agnostic wakeup: `MediaStore::progress_version()` (a monotonic
    counter) and `MediaStore::listen()` (an `event_listener::EventListener` —
    a plain `Future<Output = ()>` any executor, or none via its blocking
    `.wait()`, can drive), via the new `event-listener` dependency.
  - **`server::MediaStore::resolve_playlist`/`resolve_resource`** — the
    blocking-reload (RFC 8216bis §6.2.5.2) and part-availability decision
    logic as synchronous poll methods returning `PlaylistOutcome`
    (`Ready`/`WouldBlock`/`BadRequest`) / `ResourceOutcome`
    (`Ready`/`WouldBlock`/`NotFound`) — never blocking, never touching a
    clock. An async adapter (e.g. `multimux`) turns `WouldBlock` into an
    actual bounded wait via `MediaStore::listen()` + its own
    `tokio::time::timeout`; see the `server` module docs for the exact
    caller-driven wait-loop shape.
  - **`server::media_playlist_m3u8`/`master_playlist_m3u8`** — the LL-HLS
    playlist renderers, moved verbatim from
    `multimux::output::llhls::media_playlist_m3u8` **including the
    reentrant-lock deadlock fix** (`max_segment_duration()`/
    `target_duration_secs()` read *before* `MediaStore::with_segments_and_parts`'s
    lock).
  - **`server::CachePolicy`** (`Immutable`/`NoCache`) — the cache-control
    policy a resolved `ResourceOutcome::Ready` carries, for an adapter to
    apply as HTTP `Cache-Control`.

### Changed

- **Renamed `ll-hls-client` → `ll-hls-runtime`** (Stage 1 of the ll-hls-runtime
  unification; never published, so a free rename — no `0.1.0` behaviour
  change). The client engine moved under a `client` module
  (`ll_hls_runtime::client::LlHlsClient` etc., mirroring `rtsp-runtime`'s
  client+server split); an empty `server` module is reserved for the LL-HLS
  origin engine currently in `multimux`, to be folded in as Stage 2.

#### Client (issue #717)

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
  - `tests/golden_gate.rs` (gated on the `tokio` feature, `ffprobe`-gated,
    non-blocking CI lane — closes issue #717's last acceptance box,
    "Integrated into #569's golden-gate harness as the reference client"):
    `TokioClient` is now the **reference client** in the #569 player-validated
    golden gate. `transmux/tests/golden_gate.rs` (#569) validates only the
    origin half — transmux's own muxer output handed to an independent
    decoder (`ffprobe`). This closes the other half: demuxes the workspace's
    real `fixtures/ts/h264_aac.ts` capture (Main profile, 320x240, 25fps, 75
    real video frames) via `TsDemux`, live-paces those real samples through
    the same `LlHlsSegmenter`/`MediaStore`/`LlHlsOutput` origin stack
    `glass_to_glass.rs` uses, drives a real `TokioClient` against it over
    loopback HTTP, then muxes the **client's own** reconstructed init +
    samples (not the origin's) into a real fMP4 and hands that to `ffprobe`:
    asserts it decodes as H.264 at the source's resolution, and that
    `ffprobe -count_frames`'s own decoded frame count exactly matches the
    frames fed in — catching a drop/duplicate/reorder that corrupts the
    bitstream even when the container alone still looks well-formed. Also
    covers the non-LL/full-segment fallback path decoding correctly, and a
    self-test (`dropped_sample_changes_the_decoded_frame_count`) proving the
    frame-count oracle isn't vacuous. `.github/workflows/ci.yml`'s existing
    non-blocking `golden-gate` job now also runs this suite alongside
    transmux's.
