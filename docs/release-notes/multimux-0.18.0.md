# multimux 0.18.0

_Released 2026-11-10._

Breaking-class release (0.17 -> 0.18): the `forwarded` output-auth scheme now requires a trusted proxy, `DashWindowSegment` gains a public field, cold playlists are held until they can be played, and sibling epochs move.

Version: 0.17.0 -> 0.18.0.

## Breaking changes and migration

- `OutputAuthSpec::Forwarded` gained `trusted_proxies`; code constructing or exhaustively matching it must add the field. Behaviour: `X-Forwarded-User` is honoured only from a peer in `trusted_proxies` (default loopback); any other peer is `401`. A deployment whose proxy is not on loopback must list it.

```rust
// before
OutputAuthSpec::Forwarded { user_header, .. }
// after
OutputAuthSpec::Forwarded { user_header, trusted_proxies, .. }
```

- `DashWindowSegment` gained `public_seq` (the number a window segment is served under); struct-literal construction must set it.

- `Verifier::forwarded` is removed upstream; use `Verifier::forwarded_from(user_header, forwarded_for_header, trusted_proxies)`.

## Changed behaviour

- **Cold playlists now open in players that ignore Partial Segments (#1754).** A media-playlist request without `_HLS_msn`/`_HLS_part` made before the playlist holds a closed segment is held, through the blocking-reload wait and its timeout, until the first segment closes, then answered with at least one `EXTINF`; after the timeout it is `503` with `Retry-After`. ffmpeg, VLC, Frigate and go2rtc used to fail to open a cold stream. A request made before the route has announced its program is held too, up to the 5 s blocking-reload timeout, instead of an immediate `503`. A Low-Latency client's cold start now waits about one segment; `Config::hold_cold_playlist = false` (or `RouteHandle::with_hold_cold_playlist(false)`) restores the old immediate answers.
- A request carrying more than one `Authorization` header is answered `400 Bad Request` by the output-auth and admin gates; it used to authenticate on the first header and ignore the rest.
- The DVR also polls when the route's newest fMP4 init differs from the one it archived, so an init change rolls the period without waiting for the next closed segment.

## Added

- `OutputAuthSpec::Forwarded::trusted_proxies`: IPs and CIDR blocks allowed to set the forwarded-user header; omitted means loopback only (`127.0.0.0/8` and `::1`).
- `Config::hold_cold_playlist` (default `true`) and `RouteHandle::with_hold_cold_playlist`.
- `DvrRecorder::needs_poll_for_init`, `DashWindowSegment::public_seq`, `ProgramSegmenter::continue_after_reconnect`, and `DvrRecorder::poll_and_persist_inits` with `InitGeneration`.

## Fixed

- DASH, LL-DASH and Smooth answered `404` for every segment after a source reconnect; the MPD `startNumber` and Smooth fragment names now use the same public segment numbers the HLS playlist serves.
- DVR: a period roll that fails to open the new file no longer skips a period number; segments closed under one fMP4 init are archived under that init, not the next; segments closed but not yet archived survive a source reconnect; an evicted period's `pN.event.json` sidecar is removed with it.
- The catch-up playlist inserts `EXT-X-DISCONTINUITY` where the live tail does not continue the archive's numbering.
- The push `ReconnectEngine` resets its backoff only after a healthy connection, so a destination that accepts and then drops is backed off and eventually given up on.
- A catch-up request dropped mid-scan keeps its scan permit until the scan ends, so aborted requests cannot exceed `MAX_CONCURRENT_SCANS`.
- The HLS, DASH and Smooth pull sources take a fetch slot only when the fetch is due, so a sleeping retry or paced reload no longer holds back ready segment fetches.
- A blank or whitespace-only Bearer token is rejected at config validation (`output_auth.token`, `routes.input.auth.bearer_token`); `Authorization: Bearer ` used to authenticate.

## Other changes

- LL-HLS and TS-HLS share one HLS media-playlist implementation.
- The test-only feature is spelled `test-seams` everywhere (`webrtc-runtime/test-support` becomes `webrtc-runtime/test-seams`).
- The DASH period stitcher converts microseconds to ticks with `broadcast_common::timescale::from_micros`; rounding is unchanged.

## Dependencies

Sibling requirements move to this wave's new epochs: `transmux` 0.30 -> 0.31, `broadcast-hls` 0.6 -> 0.7, `hls-runtime` 0.12 -> 0.13, `dash-runtime` 0.3 -> 0.4, `rtsp-runtime` 0.10 -> 0.11, `broadcast-client` 0.4 -> 0.5, `broadcast-auth` 0.5 -> 0.6, `webrtc-runtime` 0.6 -> 0.7, `media-plane` 0.10 -> 0.11, `timed-metadata` 0.9 -> 0.10, `caption-convert` 0.5 -> 0.6. Patch moves: `broadcast-common` 11.0.1, `dvb-si` 13.1.0, `mpeg-ts` 0.7.1, `container-probe` 0.3.1, `rtmp-runtime` 0.9.1, `srt-runtime` 0.8.1.

---

Published from tag `multimux-v0.18.0`.
