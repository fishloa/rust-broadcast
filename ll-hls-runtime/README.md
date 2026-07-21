# ll-hls-runtime

A sans-IO Low-Latency HLS (**RFC 8216bis**, HTTP Live Streaming 2nd Edition)
**client + server** engine in one crate, mirroring `rtsp-runtime`'s
client+server split (renamed from `ll-hls-client`, never published — Stage 1
of the ll-hls-runtime unification). The crate holds two halves:

- **`client`** (issue [#717](https://github.com/fishloa/rust-broadcast/issues/717),
  slices 2-4: reload scheduler, fetch pipeline, output adapter, plus slice
  5's `tokio` feature for the async IO adapter) — a driveable LL-HLS
  playback client engine.
- **`server`** (feature `std`; issue #663/#717 Stage 2) — the sans-IO LL-HLS
  **origin engine**, moved out of `multimux`: the rolling in-RAM window
  (`server::MediaStore`), the blocking-reload + part-availability decision
  logic (`resolve_playlist`/`resolve_resource`, never blocking, never
  touching a clock), and the playlist renderers. `multimux` is now a thin
  tokio+axum adapter driving this engine; any other HTTP framework can
  adapt it the same way.

`client::LlHlsClient` is a driveable, caller-driven state machine in the same
sans-IO shape as [`srt-runtime`](../srt-runtime) (issue #565): the core never
opens a socket or reads a clock. The caller drains `Action`s (what to fetch,
and how), performs the IO itself, and feeds the response back in; decoded
`Output`s (the init segment, then ordered access units) drain out in turn.

## What's here

- **Reload scheduler** — once a playlist advertises `EXT-X-SERVER-CONTROL`/
  `EXT-X-PART-INF` (Low-Latency), every reload is a Blocking Playlist Reload
  (RFC 8216bis §6.2.5.2) naming the next not-yet-seen Partial Segment's
  `_HLS_msn`/`_HLS_part` — distinguishing a bare `_HLS_msn` (waits for a
  *closed* segment) from `_HLS_part=0`. Non-LL origins get a plain GET paced
  by a `WaitMs` hint derived from `#EXT-X-TARGETDURATION`. `EXT-X-SKIP`/
  `CAN-SKIP-UNTIL` Playlist Delta Updates are requested once a full-playlist
  baseline exists and merged back into a full view.
- **Fetch pipeline** — the `EXT-X-PRELOAD-HINT`ed part is prefetched ahead of
  its own numbered `EXT-X-PART` appearance; `BYTERANGE` parts are supported,
  including the "omitted offset continues the previous sub-range" rule; the
  Media Initialization Section (`EXT-X-MAP`) is fetched once and reused.
- **Dedup / coalescing** — once any of a segment's parts have been
  individually fetched, the segment is never re-fetched whole when it later
  closes; a playlist whose segments carry no parts at all (non-LL) falls back
  to whole-segment fetches. A part's samples are never double-counted against
  its parent segment's.
- **Output adapter** — exactly one `Output::Init` precedes any
  `Output::Samples`; parts/segments are demuxed into real access units via
  `transmux::Fmp4Demux` (never opaque container bytes); `EXT-X-DISCONTINUITY`
  surfaces as `Output::Discontinuity`; resources arriving before the init
  segment are buffered and replayed once it arrives, so a caller's fetches may
  complete in any order.

## Reuse, not re-description

The `client` module defines **no playlist model of its own**. Parsing is
`transmux::hls::MediaPlaylist::parse` (issue #717 slice 1 — the symmetric
inverse of the LL-HLS origin's own `to_m3u8()` renderer, so origin and client
share one wire model); demuxing a fetched CMAF part or segment into access
units is `transmux::Fmp4Demux`. `client` holds only the client **engine**.

## Zero IO in the core

No `tokio`/`reqwest`/socket dependency in `LlHlsClient` itself, ever. `no_std`
+ `alloc` (default `std` feature can be turned off) — verified by the
`--no-default-features` gate. Drive it by hand — see `tests/origin_loop.rs`
for a complete in-process example against a real
`transmux::ll_hls::LlHlsSegmenter` origin (no real sockets: the origin's
playlist/part/segment bytes are handed to the client exactly as a caller's
HTTP fetch loop would).

## The `tokio` feature (issue #717 slice 5)

Enabling the (non-default) `tokio` cargo feature adds `client::tokio_client::TokioClient`:
a thin async shell (tokio + reqwest/rustls) driving `LlHlsClient` over real
HTTP — blocking-reload/preload-hint query params, `Range` byte-ranges,
per-request timeouts, and retry/backoff on transient failures. Authenticates
via the shared [`broadcast-auth`](../broadcast-auth) crate
(`TokioClientConfig::auth` takes a `broadcast_auth::Credentials` —
Basic/Digest/Bearer, with Digest computed end-to-end on a `401`), the same
model `rtsp-runtime` and `multimux`'s HTTP input adapters use.
`tests/glass_to_glass.rs` (gated on this
feature) drives it against a **real** `multimux`-served LL-HLS origin over
loopback HTTP, fed by a real-time-paced synthetic producer, and measures
sub-second glass-to-glass latency — the epic's headline acceptance bar — plus
asserts blocking-reload and preload-hint prefetch are actually exercised, and
that a genuinely non-LL origin still plays via the full-segment fallback.

## The `server` module (feature `std`)

The sans-IO LL-HLS **origin engine**, moved out of `multimux` (issue
#663/#717 Stage 2) so any HTTP framework can adapt it:

- **`server::MediaStore`** — the protocol-neutral rolling in-RAM window
  (init/segments/live parts/health/max-segment-duration). Wakeup is
  runtime-agnostic: `progress_version()` (a monotonic counter) +
  `listen()` (an `event_listener::EventListener` any executor — or none, via
  its blocking `.wait()` — can drive), not a `tokio`-specific channel.
- **`server::MediaStore::resolve_playlist`/`resolve_resource`** — the
  Blocking Playlist Reload (RFC 8216bis §6.2.5.2) and part-availability
  decision logic as synchronous poll methods returning `PlaylistOutcome`/
  `ResourceOutcome` — never blocking, never touching a clock. An async
  adapter (e.g. `multimux`) turns a `WouldBlock` outcome into an actual
  bounded wait via `MediaStore::listen()` plus its own `tokio::time::timeout`
  (or equivalent).
- **`server::media_playlist_m3u8`/`master_playlist_m3u8`** — the LL-HLS
  playlist renderers; `master_playlist_m3u8` takes the media playlist's
  served filename as an explicit argument, so an adapter can serve it under
  any configured name.
- **`server::CachePolicy`** (`Immutable`/`NoCache`) — the cache-control
  policy a resolved `ResourceOutcome::Ready` carries, for an adapter to
  apply as HTTP `Cache-Control`.

`multimux` is the reference adapter: a thin tokio+axum layer that calls
`resolve_playlist`/`resolve_resource` and drives the one thing the sans-IO
engine can't — the actual bounded `.await` on `WouldBlock`.

## What's *not* here — explicit follow-ups

- **Multivariant Playlist rendition selection** — `transmux::hls::MasterPlaylist::parse`
  exists, but choosing a rendition/bitrate is a player-level policy this crate
  doesn't impose; `LlHlsClient` follows one Media Playlist URL.
- **Discontinuity signalling on a still-open segment** — RFC 8216bis's
  `OpenSegment` (the in-progress, not-yet-closed segment) carries no
  discontinuity flag of its own; if every part of a segment was already
  delivered while it was open, a discontinuity revealed only once it closes is
  signalled late (after those parts' samples). A gap in the current wire
  model, not something this crate can fix locally.

```toml
[dependencies]
ll-hls-runtime = "0.1"
```

## License

MIT OR Apache-2.0.
