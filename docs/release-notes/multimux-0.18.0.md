---
status: DRAFT
---
# multimux 0.18.0

_Released 2026-11-10._

Breaking-class release (0.17 -> 0.18): the `forwarded` output-auth scheme now requires a trusted proxy, and `DashWindowSegment` gains a public field; sibling epochs move.

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

## Added

- `OutputAuthSpec::Forwarded::trusted_proxies`: IPs/CIDR blocks allowed to set the forwarded-user header; omitted means loopback only.
- `Config::hold_cold_playlist` (default `true`) and `RouteHandle::with_hold_cold_playlist`.
- `DvrRecorder::needs_poll_for_init`, `ProgramSegmenter::continue_after_reconnect`, `DvrRecorder::poll_and_persist_inits` with `InitGeneration`.

## Fixed

- DASH, LL-DASH and Smooth `404` after source reconnect resolved (shared public segment numbers).
- DVR period roll opens new file first; archived pending segments under correct init; evicted period sidecars removed.
- Push `ReconnectEngine` backoff only resets after a healthy connection.
- Catch-up scan permit moves into blocking task.
- HLS/DASH/Smooth fetch scheduler slots taken only when due.

## Other changes

- LL-HLS and TS-HLS share one HLS media-playlist implementation.
- Test-only feature spelling unified to `test-seams`.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-auth` 0.5 -> 0.6
- `webrtc-runtime` 0.6 -> 0.7
- `rtsp-runtime` 0.10 -> 0.11
- `broadcast-client` 0.4 -> 0.5
- `dash-runtime` 0.3 -> 0.4

---

Published from tag `multimux-v0.18.0`.
