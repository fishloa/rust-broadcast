# ll-hls-client

A sans-IO Low-Latency HLS (**RFC 8216bis**, HTTP Live Streaming 2nd Edition)
playback client engine — issue [#717](https://github.com/fishloa/rust-broadcast/issues/717),
slices 2-4 (reload scheduler, fetch pipeline, output adapter).

`LlHlsClient` is a driveable, caller-driven state machine in the same sans-IO
shape as [`srt-runtime`](../srt-runtime) (issue #565): the core never opens a
socket or reads a clock. The caller drains `Action`s (what to fetch, and how),
performs the IO itself, and feeds the response back in; decoded `Output`s (the
init segment, then ordered access units) drain out in turn.

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

This crate defines **no playlist model of its own**. Parsing is
`transmux::hls::MediaPlaylist::parse` (issue #717 slice 1 — the symmetric
inverse of the LL-HLS origin's own `to_m3u8()` renderer, so origin and client
share one wire model); demuxing a fetched CMAF part or segment into access
units is `transmux::Fmp4Demux`. `ll-hls-client` holds only the client
**engine**.

## Zero IO in the core

No `tokio`/`reqwest`/socket dependency anywhere in this crate. `no_std` +
`alloc` (default `std` feature can be turned off). A real deployment wraps
`LlHlsClient` in an HTTP-fetching loop — planned as a follow-up slice
(mirroring `srt-runtime`'s `io` module / `rtsp-runtime`'s `tokio` feature).
Until then, drive it by hand — see `tests/origin_loop.rs` for a complete
in-process example against a real `transmux::ll_hls::LlHlsSegmenter` origin
(no real sockets: the origin's playlist/part/segment bytes are handed to the
client exactly as a caller's HTTP fetch loop would).

## What's *not* here — explicit follow-ups

- **The async IO adapter** (tokio + an HTTP client) that actually performs the
  `Action`s and feeds responses back — the sans-IO core this crate ships is
  the reusable engine; wiring it to real sockets is the next slice.
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
ll-hls-client = "0.1"
```

## License

MIT OR Apache-2.0.
