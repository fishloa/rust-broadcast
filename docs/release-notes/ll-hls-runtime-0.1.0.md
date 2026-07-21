# ll-hls-runtime 0.1.0 — 2026-07-21

First publish (as `ll-hls-runtime`; renamed from the never-published
`ll-hls-client`). A sans-IO **Low-Latency HLS client + server** in one
crate — RFC 8216bis — mirroring `rtsp-runtime`'s client+server split (issue
#663/#717, the multimux-hub epic).

## What it is

- **`client`** — `LlHlsClient`, a caller-driven playback state machine in
  the same sans-IO shape as `srt-runtime`: `poll()`/`next_output()` drain
  queued `Action`s/`Output`s; `on_playlist()`/`on_resource()`/`on_error()`
  feed responses back in. No socket, no clock, no `tokio`/`reqwest`
  dependency in the core. Covers the Blocking Playlist Reload scheduler
  (`_HLS_msn`/`_HLS_part`, RFC 8216bis §6.2.5.2), `EXT-X-PRELOAD-HINT` part
  prefetch, `BYTERANGE` part/segment/map support, and an output adapter that
  emits ordered `Output::Init`/`Output::Samples` (real access units via
  `transmux::Fmp4Demux`) plus `Output::Discontinuity`. Reuses
  `transmux::hls::MediaPlaylist::parse` for the playlist model — this crate
  defines no playlist types of its own.
  - `client::tokio_client::TokioClient` (feature `tokio`) drives the client
    over real HTTP via `reqwest` (rustls), including the blocking reload,
    resource fetches with capped-backoff retry, and (this release)
    authenticates via `broadcast-auth` (Basic/Digest/Bearer) rather than an
    ad hoc Basic/Bearer-only `Auth` enum — Digest now works end-to-end.
- **`server`** (feature `std`) — the sans-IO LL-HLS **origin engine**,
  moved out of `multimux` so any HTTP framework can adapt it, not just
  `multimux`'s tokio+axum stack:
  - `server::MediaStore` — the protocol-neutral rolling in-RAM window
    (init/segments/live parts/health/max-segment-duration), with a
    runtime-agnostic wakeup (`progress_version()`/`listen()` via
    `event-listener`) instead of a `tokio`-specific watch channel.
  - `server::MediaStore::resolve_playlist`/`resolve_resource` — the
    blocking-reload and part-availability decision logic as synchronous,
    never-blocking poll methods returning `PlaylistOutcome`/
    `ResourceOutcome`; an async adapter turns `WouldBlock` into an actual
    bounded wait via `listen()` + its own timeout.
  - `server::media_playlist_m3u8`/`master_playlist_m3u8` — the LL-HLS
    playlist renderers (the latter now takes the media-playlist filename as
    an explicit argument, so a server can serve it under any configured
    name).
  - `server::CachePolicy` (`Immutable`/`NoCache`) for an adapter's
    `Cache-Control` header.

## Testing

`tests/origin_loop.rs` — an in-process origin↔client loop against a real
`transmux::ll_hls::LlHlsSegmenter`. `tests/glass_to_glass.rs` (feature
`tokio`) drives `TokioClient` against a real `multimux`-served LL-HLS
origin over real loopback HTTP and asserts sub-second glass-to-glass
latency. `tests/golden_gate.rs` (feature `tokio`, `ffprobe`-gated) makes
`TokioClient` the reference client in the workspace's player-validated
golden gate: demuxes a real capture, live-paces it through the origin
stack, drives a real client against it, and hands the client's own
reconstructed samples to `ffprobe` to verify frame-exact decode.

## Fixed

`MediaStore::window_segments`/`last_closed_segment_seq` no longer use a
bare `.lock().unwrap()` — a pre-release audit finding; both now tolerate a
poisoned `Mutex` like the store's other lock sites.

## Compatibility

MSRV 1.86. `client` core is dependency-light (no `tokio`/`reqwest`); the
`tokio` feature pulls those plus `broadcast-auth`; `server` needs the `std`
feature (`std::sync::Mutex`).
