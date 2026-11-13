# broadcast-client 0.5.0

_Released 2026-11-14._

Breaking-class release (0.4 -> 0.5): breaking API changes are listed below, and the crate's sibling dependencies move to new caret epochs.

Version: 0.4.0 -> 0.5.0.

## Breaking changes and migration

- `Session` is no longer `UnwindSafe` or `RefUnwindSafe`, and `tshttp::{TsHttpClient, TsHttpPolicy, TsHttpSource}` are no longer `RefUnwindSafe` (they now hold state that is not unwind-safe, such as the shared `RetrySchedule`'s jitter `Cell`); code that moves them across `std::panic::catch_unwind` must wrap them in `AssertUnwindSafe`.

- The test-only feature is renamed `__test-seams` -> `test-seams`.

```toml
# before
broadcast-client = { version = "0.4", features = ["__test-seams"] }
# after
broadcast-client = { version = "0.5", features = ["test-seams"] }
```

## Fixed

- Join-point audio end comparison uses `clock33::pcr_signed_distance`.
- Every sample's end reported to buffer controller in same call.
- RTSP/RTMP connection loss hinted as `SourceEnded`/`SourceError`.
- Live after finite connection keeps stall memory and pending-track placeholder.
- Protocol-error hint, `buffer_input_log_seed`, contiguous video media end reporting.

## Other changes

- Test-only feature spelling unified.

## Also in this release

### Changed
- Smooth sessions jitter their fragment and manifest retries like every other source: the session passes its jitter seed to the engine (`SmoothConfig::jitter_seed`); the first retry still waits exactly its step (#2589).
- URL userinfo is read with `broadcast_auth::credentials_from_url` and removed with `strip_userinfo`; the private duplicate (`take_userinfo`) is deleted. Behaviour is unchanged for `user:pass@`; the control-character check still applies (#2587).
- `Session::payload_bytes()` also counts every response body byte of an HLS, DASH or Smooth session.
- An RTSP session announces its `Event::Tracks` (from the DESCRIBE SDP) when PLAY is answered instead of with the first access unit, so a host sees the programme before any media arrives.
- `SrtConfig::latency_ms` accepts any value: a latency below 120 ms is no longer an `InvalidConfig` (the negotiated latency is the larger of the two peers' values).
- `Retry-After` HTTP-date parsing now uses `broadcast-common::http_date::parse_http_date`.
- `AuthState` now comes from `broadcast-auth`; `broadcast-client` keeps no copy.
- RTSP keepalive now uses `rtsp_runtime::ClientSession`'s schedule (half the declared `Session;timeout=`, the RFC 2326 §12.37 60 s default when none is declared) instead of a local timer with its own default and ceiling, and its method is the one the server advertises.
- RTP sequence validity now uses `rtp_packet::SeqValidity` instead of a local delta/resync heuristic.
- `source::fetch::wall::WallClock` is `broadcast_common::time::WallClock` (the shared `time::MonoUtc` anchor mapping) rather than a second copy of the same arithmetic; `Session::set_wall_clock` and the `Retry-After` reading are unchanged.
- RTSP pull uses rtsp-runtime's `MAX_TRACKS` cap instead of a local constant.
- RTSP pull skips an SDP media section whose `m=` format is not a numeric RTP payload type (RFC 4566 §5.14) instead of playing it, matching multimux; the skip is reported in `skipped`.
- Timeline-jump detection now uses the detector moved to `transmux` (re-exported). `Config::timeline_jump` is now `Option<transmux::TimelineJumpConfig>`; the `broadcast_client::TimelineJumpConfig` re-export remains, but the type path changed (breaking for users naming `broadcast_client::timeline::TimelineJumpConfig`). `timeline::DEFAULT_JUMP_BACKWARD_US` and `DEFAULT_JUMP_FORWARD_SLACK_US` are removed from broadcast-client; they are `transmux::timeline::DEFAULT_JUMP_BACKWARD_US` / `DEFAULT_JUMP_FORWARD_SLACK_US` (#2469).
- The RTSP source reads each DESCRIBE media section through `transmux::sdp_track`; its own `parse_tracks`/`supported_payload` interpreter is deleted (output unchanged, pinned by `fixtures/golden/sdp-tracks/broadcast-client.txt`).
- The Smooth source builds `smooth_runtime::FetchFailure` with its new `retry_after_us` field left at `0` (#2393).
- The RTSP track aligner (Sender Report / `RTP-Info` / provisional placement) is `transmux::TrackAligner`, shared with multimux's RTSP pull; `AvSync` is now `transmux::AvSync`, re-exported unchanged from `broadcast_client` (#2380).
- BREAKING: the Smooth source is an adapter over the new `smooth-runtime` engine (#2370); the chunk planner, refresh, retry and track logic it carried are gone. Visible changes: the audio track is announced with its own id (`Event::Tracks` is `[1, 2]`, not `[1, 1]`, and audio samples carry track 2); a live manifest refresh no longer re-fetches the last fragment; a fragment that fails to demux is retried like any failed fetch instead of being dropped silently; the live refresh wait is half the last fragment's duration in microseconds, clamped to 0.5 s-5 s.
- The RTSP source plans its `SETUP`/`PLAY` through `rtsp_runtime::PlayPlan` (control resolution against `Content-Base`, channel pairs, the RTP/RTCP collision check) instead of its own copy; the wire behaviour is unchanged (#2372).
- BREAKING: the Smooth source parses the client Manifest with `broadcast-smooth`'s `SmoothManifest` instead of `transmux`'s removed `smooth_parse` module (#2356); `transmux::track_spec_from_quality_level` still builds the track spec.

### Added
- `skills/broadcast-client-basics/SKILL.md`: a Symposium agent skill shipped in the published package, so an AI coding agent working in a project that depends on `broadcast-client` gets usage guidance covering the host loop over `Session`, `Config`, source selection, auth/cookies/retry, the event queue limits, features and the common pitfalls.
- `Config::with_timeouts(Timeouts)` and `tshttp::Config::with_timeouts`: one `Timeouts` mapped onto the session's bounds by `Config::kind`, with the per-protocol meaning documented once (HLS: `read_idle` bounds playlist/segment/key requests; DASH and Smooth: manifest and segment requests; SRT: `connect` is the handshake bound; every other kind: `connect` is the stall policy's start timeout and `read_idle` its reconnect threshold). Every bound is floored at 1 ms (#2590).
- `fetch::ReconnectTimer` (`new(steps_ms, cap_ms, seed)`, `on_connected`, `on_disconnected`, `attempts`, `reset`) and `fetch::HEALTHY_CONNECTION_US` (30 s): the sans-IO reconnect timer, driven by the caller's clock, that broadcast-client and multimux share; the backoff and attempt counter restart only after a connection lasted at least `HEALTHY_CONNECTION_US` (#2471, #2474).
- `SrtConfig::timeouts` and `SrtConfig::with_timeouts`: the SRT handshake and peer-idle bounds read and set as `broadcast_common::timeouts::Timeouts` (`handshake`, `read_idle`), so a host maps its own timeouts once instead of converting to the `_ms` fields (#2514).
- `Config::max_queued_bytes` (default 256 MiB, `DEFAULT_MAX_QUEUED_BYTES`; `0` is `InvalidConfig`): a byte budget on the queued `Event::Sample` payloads next to `max_queued_events`. A host that does not drain loses the oldest samples (counted by `dropped_events`, each burst leaving one `Event::Discontinuity`) once either bound is exceeded.
- `Config::utc_timing_credentials` (on by default): whether a DASH session sends its credentials to a `UTCTiming` server on the session origin; a relay turns it off.
- `Config::preemptive_basic` (off by default): send `Basic` credentials on the first request to the session origin instead of waiting for a `401` (RFC 7617).
- `ErrorKind::refusal_status()` (the HTTP status behind a failed key, init or segment request) and `ErrorKind::is_permanent()` (content or codec errors a retry cannot fix).
- `UdpConfig::sdp`: an out-of-band SDP (RFC 4566) that switches an `rtp://` session from MPEG-2 TS (payload type 33) to the H.264 (RFC 6184) and AAC (RFC 3640) streams it declares, routed by RTP payload type and depayloaded with `transmux`'s depacketisers. The tracks are announced when the socket is bound, before any packet; a reconnect flushes the access units still held. `datagram_stats()` counts the datagrams and the ones that were not RTP, and `payload_bytes()` the packets routed to a declared stream.
- `ErrorKind::RequestRefused { method, status }` (non-fatal, also `ends_connection()`): an RTSP server's DESCRIBE, SETUP or PLAY answer the session cannot proceed on, with the method and the status as the server sent it.
- `Session::payload_bytes()`: transport-stream payload bytes accepted from the source, excluding control traffic and dropped datagrams, for hosts that measure stalls.
- `ErrorKind::SrtPeerIdle` (non-fatal) when the SRT peer goes silent past `peer_idle_timeout_ms`, and `ErrorKind::ends_connection()` naming the errors that end the connection the session was reading.
- `UdpConfig::interface_index` and `MulticastJoin::interface_index`: join an IPv6 multicast group on an interface named by index (valid only for an IPv6 group, not together with `interface`).
- `Config::relay()`: a preset for a host that forwards samples instead of playing them (no start-pending wait, no timeline-jump screening; the initial-tracks hold stays); `Event::Tracks` specs keep their `program_number`, which a relay uses to split a multi-programme stream.
- `broadcast_client::auth` is public: `AuthState` (origin-scoped credentials, challenge handling, `Clone`, `credentials()`) with `AuthDecision`, and `AuthState::preauthorize_basic` to send Basic on the first request (RFC 7617) instead of after a `401`.
- `broadcast_client::fetch::retry_after_us` is public sans-IO retry/backoff policy (RFC 9110 §10.2.3), for use by other pull hosts.

### Removed
- BREAKING: `broadcast_client::auth`'s own `AuthState`, `AuthDecision`, `AuthEvent` and `carries_credentials` definitions — they live in `broadcast-auth` now, and this module re-exports them, so the paths keep working but the types are `broadcast_auth`'s.

### Fixed
- RTMP URLs are parsed by `rtmp_runtime::target::RtmpTarget` (the second parser in `source/rtmp.rs` and the duplicated default-port constants are deleted); behaviour follows rtmp-runtime's grammar: `rtmps` default port 443, percent-decoded path (#2588).
- A `206` `Content-Range` is parsed by one RFC 9110 §14.4 parser (`broadcast_common::content_range`) instead of three: `bytes 5-20/10` is refused for a ranged fetch (it was accepted), `bytes 0-9/*` is a finite ts/http body of 10 bytes (it was not), and stray whitespace inside the value is invalid (ts/http accepted it) (#2586).
- A redirect `Location` (HTTP and RTSP) carrying a control character or space is no longer followed: it resolves through `broadcast_common::url_ref::resolve` instead of the raw `Url::join`, which deleted a TAB/CR/LF and followed a different URL than the header names (RFC 3986 §2) (#2585).
- A Smooth quality switch to a level with a different SPS/PPS or resolution now succeeds (it was refused as `NoSuchVariant`): the engine's `Output::TracksChanged` surfaces as a new `Event::Tracks`, the event DASH variant switches already use.
- `Session::rtcp_rejected_count`: the RTSP source's Sender Reports that were unusable (NTP time zero, or a set refused for tracks more than an hour apart in wallclock), which were previously counted nowhere a caller could read.
- The RTSP source flushes its track aligner when a connection ends, drops or is reconnected, so samples held back while waiting for the video anchor are delivered instead of lost (audit 7 A7-4).
- `Session::select_variant` on a Smooth source switches the video `QualityLevel` of the fragments requested afterwards; it used to validate the index and return `Ok` without changing anything.
- A DASH source whose MPD is `MPD@type="list"` (ISO/IEC 23009-1 6th edition) fails with the typed unsupported-addressing error instead of a generic playback-timeline failure (#2340).

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-auth` 0.6 -> 0.7, `broadcast-common` 11 -> 12, `broadcast-hls` 0.6 -> 0.7, `buffering-runtime` 0.2 -> 0.3, `dash-runtime` 0.3 -> 0.4, `dvb-si` 13 -> 14, `hls-runtime` 0.12 -> 0.13, `mpeg-ts` 0.7 -> 0.8, `rtmp-runtime` 0.9 -> 0.10, `rtp-packet` 0.6 -> 0.7, `rtsp-runtime` 0.10 -> 0.11, `srt-runtime` 0.8 -> 0.9, `transmux` 0.30 -> 0.31.
- New sibling dependencies: `smooth-runtime` 0.1.

---

Published from tag `broadcast-client-v0.5.0`.
