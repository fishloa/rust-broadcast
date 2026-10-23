# broadcast-client

Generic sans-IO client session for live media: auth, redirects, reconnect/retry and buffering
decisions over a TS-over-HTTP source (`tshttp-runtime`) and an HLS source (`hls-runtime`).
The host performs only the HTTP transport; time is a caller-supplied monotonic `now_us`.

## Sources

`Config::kind` selects the source; `SourceKind::Auto` picks HLS for a `.m3u8`/`.m3u` URL path
(ASCII case-insensitive) and TS-over-HTTP otherwise.

HLS (BC-3a): a single media playlist, or a master playlist with the variant chosen by
`Config::variant_policy` (`Event::Variants` reports the choice, `Session::select_variant` switches
and emits `Event::Discontinuity`). TS and fMP4 segments, VOD (`Status::Ended` once) and live
(blocking reload). Every request gets the origin-scoped auth layer and its own redirect budget.
Failed requests retry with the `Config::hls_backoff_ms` schedule (2, 4, 8 s, then
`hls_backoff_cap_ms`, 10 s) and report `Status::Reconnecting`; a 404/410 playlist also reports
`Status::NotFound`. Bodies are bounded (`max_playlist_bytes`, `max_segment_bytes`) and requests
time out (`hls_request_timeout_us`, `hls_segment_timeout_us`). Separate audio/subtitle renditions
and `Event::Buffer` for HLS are not implemented.

## Quick start

```rust
use broadcast_client::{Config, Session};

let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
while let Some(action) = session.poll(0) {
    // host performs I/O based on `action`
    let _ = action;
}
```
