# Changelog

All notable changes to `broadcast-client` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Initial release. A generic sans-IO client session for live and on-demand media: the host performs only transport I/O (HTTP requests, or a TCP connection for RTSP) and reports what happened; the session owns auth, redirects, cookies, reconnect/retry, demuxing and buffering decisions. Time is a caller-supplied monotonic `now_us`. Design: `docs/broadcast-client/DESIGN.md`.

### Added

- `Session`: `open`/`open_with_memory`, `poll`/`next_event`, `on_response`/`on_data`/`on_closed`/`on_connected`, `on_playback_position`, `set_credentials`, `cookies`, `reconnect`, `rejoin`, `select_variant`, `select_audio_rendition`, `set_rendered_tracks`/`default_rendered_tracks`, `hint`, `stall_memory`, `close`, and the counters `rtp_loss_count`, `rtp_malformed_count`, `rtcp_packet_count`, `stale_samples_dropped`, `dropped_events`.
- Three sources behind one API, chosen by `Config::kind` (`SourceKind::{Auto, TsHttp, Hls, Rtsp}`; `Auto` picks RTSP for `rtsp://`/`rtsps://`, HLS for a `.m3u8`/`.m3u` path, TS-over-HTTP otherwise):
  - TS-over-HTTP over `tshttp-runtime`: reconnect/backoff, start and stall timers, finite (`Content-Length`) detection, and a track-announcement hold that releases the first media behind ONE `Event::Tracks` (`tracks_hold_*`, `start_pending_track_wait_us`).
  - HLS over `hls-runtime`: media and master playlists, variant selection (`VariantPolicy`, `Event::Variants`, `select_variant`), separate audio renditions (`auto_select_audio`, `preferred_audio_language`, `select_audio_rendition`, `Event::AudioRendition`, ids offset by `AUDIO_TRACK_ID_OFFSET`), TS and fMP4 segments, VOD (`Status::Ended`) and live blocking reload, byte ranges, `AES-128` segment decryption with a cached key, `Retry-After` (seconds form), per-segment retry accounting, a bounded number of in-flight segment requests, and a per-run `EXT-X-MAP` init.
  - RTSP over `rtsp-runtime` (SAT>IP tuners, IP cameras): DESCRIBE, one SETUP per SDP media, PLAY, RTP/RTCP interleaved over the RTSP TCP connection (`rtsp`/`rtsps` via `Action::TcpConnect { tls }`), H.264 video and AAC audio, keepalive at half the server `Session` timeout, redirects, `TEARDOWN` on close.
- Auth layer over `broadcast-auth` (Basic, Digest, Bearer): `Event::AuthRequired` / `AuthRejected` / `Forbidden`, `Status::WaitingForCredentials`, credentials in URL userinfo (percent-decoded, stripped from request URIs), origin-scoped credentials, stale-Digest replay once, `Error::InvalidCredentials` on control characters.
- Redirects (`Config::max_redirects`): followed to `http`/`https` (`rtsp`/`rtsps` for RTSP) only, never downgraded, credentials dropped across authorities, Digest `uri=` per RFC 7616 section 3.4.
- Cookie jar (`Config::initial_cookies`, `Session::cookies`, `docs/cookies.md`) scoped to the session origin with `Path`, `Secure`, `Max-Age`, `__Secure-`/`__Host-` rules and size caps; `Config::extra_headers` validated at open (`Error::InvalidHeader`, `Error::ReservedHeader`).
- `Event::Buffer(Start|Pause|Resume)` from two engines: live TS-over-HTTP by `buffering-runtime` (`Config::buffer`), and HLS plus finite TS by a deterministic playout gate (`Config::playout`, `PlayoutConfig`: `vod_start_lead_us`, `live_start_lead_us`, `resume_lead_us`, `pause_lead_us`).
- Timeline-jump detection (`Config::timeline_jump`, `TimelineJumpConfig { backward_us, forward_slack_us }`, on by default): a new timeline that no discontinuity flag announces emits `Event::Discontinuity`, re-arms the join and resets the gate; stale samples of other tracks are dropped and counted.
- Join decisions on video samples (`JoinPoint` at the first random-access point, `Drop` before it, `Keep` after), NAL-aware for AVC/HEVC/VVC and sync-sample based for other video codecs.
- `Event` surface: `Tracks`, `Sample { track, sample, join }`, `DisplayChanged`, `Discontinuity`, `Finite`, `Status`, `Starved`, `Buffer`, `AuthRequired`, `AuthRejected`, `Forbidden`, `Variants`, `AudioRendition`, `Error { kind, fatal }`. `Status`: `Establishing`, `Live`, `Stalled`, `Reconnecting`, `NotFound`, `WaitingForCredentials`, `Ended`.
- `ErrorKind` (every drop or failure is a typed, labelled `Event::Error`, never silent): `UnsupportedEncryption`, `KeyFetchFailed`, `InitFetchFailed`, `InitDecodeFailed`, `SegmentFetchFailed`, `BadKey`, `DecryptFailed`, `UnsupportedCodec`, `TrackIdReserved`, `TrackDropped`, `UnsupportedContainer`, `BodyTooLarge`, `UnknownRenderedTrack`.
- `Action` surface: `HttpRequest { conn, method, url, headers, byte_range }` (`byte_range` is `(offset, length)`), `TcpConnect`, `Send`, `Close`, `WakeAt`. `Debug` never prints header values or `Send` payloads.
- `Config` knobs (all with documented defaults, validated by `Session::open` -> `Error::InvalidConfig`): `kind`, `ts_http`, `max_redirects`, `extra_headers`, `initial_cookies`, `buffer`, `playout`, `variant_policy`, `max_manifest_bytes` (4 MiB), `max_segment_bytes` (64 MiB), `request_timeout_us` (20 s), `segment_timeout_us` (60 s), `backoff_ms` (2/4/8 s), `backoff_cap_ms` (10 s), `max_inflight_segments` (4), `hls_key_timeout_us` (20 s), `max_key_bytes` (4096), `max_key_failures` (8), `max_segment_failures` (8), `preferred_audio_language`, `auto_select_audio`, `max_queued_events` (100 000), `timeline_jump`, `tracks_hold_max_input_bytes` (4 MiB), `tracks_hold_quiet_input_bytes` (512 KiB), `tracks_hold_max_media_us` (1 s), `tracks_hold_max_wall_us` (1 s), `start_pending_track_wait_us` (1.5 s).
- DASH clock and configuration surface (design `docs/dash-runtime/DESIGN.md` 3.5, 6.3, 6.4; the DASH source itself is a later step): `Session::set_wall_clock(now_us, unix_us)`, `Config::dash: DashConfig` (re-exported with `DashClockPolicy`, validated as `dash.<field>`), `ClockSource` (from `dash-runtime`), `Event::ClockSynced { source, uncertainty_us }`, and the `ErrorKind`s `MpdParse`, `MpdInvalid`, `UnsupportedAddressing`, `UnsupportedFeature`, `LiveEdgeLost`, `ClockUnavailable`. New dependency on `dash-runtime`.
- `Retry-After` in HTTP-date form (RFC 9110 5.6.7, all three forms) is honoured, counted from the `set_wall_clock` anchor, else from the response's `Date` header, clamped to 60 s; a delta-seconds value beyond `u64` is now clamped instead of ignored, and a signed value (`+5`) is no longer accepted as delta-seconds.
- Chunking invariance: the `Sample` sequence and the position of every `Discontinuity` are identical for every read size.
- Bounded event queue: transient `Status`/`Starved` coalesce, then the oldest `Sample`s are dropped (counted, each burst leaving one `Discontinuity`); tracks, errors, buffer events and terminal statuses are never dropped.
- Optional `serde` feature (`broadcast-common`, `transmux`, `tshttp-runtime`, `buffering-runtime`); `test-seams` feature for the in-tree tests (not public API).
- Examples `live_loop` and `auth_wait`; lexical no-hand-roll guard, label-coverage drift guard and `#[non_exhaustive]` coverage guard.

### Known limits

- RTSP carries H.264 video and AAC audio only; H.265 over RTP needs an RFC 7798 depayloader in `transmux` (not present), so an SDP whose only media is H.265 fails with one fatal `ErrorKind::UnsupportedCodec`. UDP RTP is not supported.
- DASH (the clock and configuration surface exists, the source does not yet), SRT, Smooth Streaming and RTMP pull sources are not included.
- HLS subtitle renditions are listed in `Event::Variants` but never fetched; CENC/CBCS and SAMPLE-AES are never decrypted (`ErrorKind::UnsupportedEncryption`).
- A `Retry-After` HTTP-date with neither a `set_wall_clock` anchor nor a response `Date` header is ignored.
- Live TS-over-HTTP: a declared audio stream that resolves later than `start_pending_track_wait_us` after the first media still produces a `Start` followed by a `Pause`.
- An HLS audio rendition is a separate client, so audio and video are aligned by the player on PTS, not at the transport level.
