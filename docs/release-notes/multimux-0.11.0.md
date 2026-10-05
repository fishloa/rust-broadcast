# multimux 0.11.0

_Released 2026-10-05._

Breaking (0.x minor) and security release of the multi-input / multi-output repackaging origin. It is three things at once: the five security fixes (listed in the table below) that were prepared as 0.11.0 but never tagged; the de-hand-roll rewrite of the HTTP stack, URL/SDP/XML/date handling, reconnect backoff and the ingest/push drivers onto maintained crates; and a large audit-fix pass over the origin, pull sources, DVR/catch-up and WHIP/WHEP. **Everyone should upgrade.** Operators running the `multimux` binary (see [multimux-cli-0.9.0.md](multimux-cli-0.9.0.md)) must read "Behaviour changes", because some JSON configs that loaded on 0.10 are now rejected and several served resources changed shape. Embedders using the library must read "Breaking API changes". Unlike what an earlier draft of this note said, **config and HTTP behaviour are not unchanged.**

Read together with: [transmux-0.25.0.md](transmux-0.25.0.md), [hls-runtime-0.7.0.md](hls-runtime-0.7.0.md), [media-plane-0.5.0.md](media-plane-0.5.0.md), [broadcast-hls-0.3.0.md](broadcast-hls-0.3.0.md), [broadcast-auth-0.4.0.md](broadcast-auth-0.4.0.md), [rtsp-runtime-0.7.0.md](rtsp-runtime-0.7.0.md), [rtmp-runtime-0.7.0.md](rtmp-runtime-0.7.0.md), [srt-runtime-0.5.0.md](srt-runtime-0.5.0.md), [webrtc-runtime-0.2.0.md](webrtc-runtime-0.2.0.md) (for `whip`/`whep`), [timed-metadata-0.6.0.md](timed-metadata-0.6.0.md).

## Security

| Area | Before this release |
|---|---|
| WHEP / WHIP HTTP | WHEP did not apply the configured `output_auth`. Sessions never ended, so the 64 slots filled permanently. The request reader had no header or body limit and no timeout. WHIP allocated a session before checking capacity. |
| WHIP / WHEP media | The DTLS peer was not authenticated against the SDP fingerprint (fixed in webrtc-runtime 0.2.0; multimux now passes the offer's `a=fingerprint`). |
| push outputs | `drive_push` parked a runtime worker thread and busy-spun when every listener slot was taken. |
| RTP/UDP input | One malformed datagram froze the route permanently while it still reported Live. |
| route ingest / output auth | A route accepted a second, concurrent RTMP/WHIP publisher and silently handed program ownership to it (or froze once the first disconnected); output-auth and admin-auth Digest challenges re-prompted on an expired nonce instead of returning `stale=true`; nothing told an operator when an ingest route ran with no authentication. |

Other hardening in this release (issue #1083 and the audit rounds behind it): a route name of `..` could make the DVR/catch-up archive escape `archive_root` (names are now restricted, see below); pull sources read whole response bodies into memory (now capped at 64 MiB, `source::MAX_HTTP_BODY_BYTES`, with a 3-hop redirect limit that refuses an `https` to `http` downgrade); push destinations (userinfo, stream keys, SRT `passphrase`/`streamid`) no longer appear in any log line, error text or `Debug` output; and `max_concurrent_requests` is now actually one server-wide bound (it was per route, method and stream).

## Breaking API changes (library users)

### Dependency epochs

`Cargo.toml` delta against 0.10.0 (verified from `git diff multimux-v0.10.0..HEAD`):

```toml
transmux        = "0.25"   # was 0.24
broadcast-hls   = "0.3"    # was 0.2
rtsp-runtime    = "0.7"    # was 0.6
rtmp-runtime    = "0.7"    # was 0.6
srt-runtime     = "0.5"    # was 0.4
hls-runtime     = "0.7"    # was 0.6
broadcast-common = "9.4"   # was 9.3
media-plane     = "0.5"    # was 0.4
broadcast-auth  = "0.4"    # was 0.3
webrtc-runtime  = "0.2"    # was 0.1 (whip / whep features)
dvb-si          = "11.0"   # was 10
mpeg-ts         = "0.5"    # was 0.4
sdp-types       = "0.2"    # was 0.1
axum            = "0.8"    # was 0.7
tower-http      = "0.7"    # was 0.5
reqwest         = "0.13"   # was 0.12 (feature rustls-tls renamed rustls)
timed-metadata  = "0.6"    # new direct dependency
```

New direct dependencies: `quick-xml 0.42`, `hyper 1` and `hyper-util 0.1` (server, http1 only), `http-body`, `headers`, `arc-swap`, `jiff 0.2`, `backon 1.6`, `parking_lot 0.12`, `socket2 0.6`. The HTTP listeners are HTTP/1 only, so the `h2` half of the stack is not used. New cargo features: `test-seams` and `test-hooks` (both off by default, below). MSRV stays 1.95.0. A crate that names `reqwest::Error`, `axum` types or the other epochs above through multimux's public API must move with it. The changelog line saying `broadcast-auth` 0.3.1 is stale: the manifest requires 0.4, and 0.3.1 was never published.

### Shutdown is a `CancellationToken`

The `watch<bool>` shutdown signal is gone. `origin::supervisor::supervise_driver` takes `cancel: tokio_util::sync::CancellationToken` and `registry::InputCtx` exposes `cancel` instead of `shutdown_rx`. Affects every `InputSpec::Custom` factory.

```rust
// 0.10
let task = tokio::spawn(supervise_driver(attempt, handle, Backoff::production_default(),
                                         ctx.name.clone(), ctx.shutdown_rx));
// 0.11
let task = tokio::spawn(supervise_driver(attempt, handle, Backoff::production_default(),
                                         ctx.name.clone(), ctx.cancel));
```

`source::run_rtsp`, `run_ts_udp`, `run_rtp_udp`, `run_ts_http`, `run_srt_caller`, `run_srt_listener_once` and `drive_socket` each take a `CancellationToken` as well. `multimux-cli` needs no change.

### Async and fallible signatures

- `source::advance_route` is now `async` (its DVR drain writes to disk on the blocking pool): add `.await`.
- `RouteHandle::add_segment` returns `Result<(), AddSegmentError>` (variants `ProgramNotPublished`, `WriterUnavailable`, `Publish`) instead of logging and dropping.
- `source::file_reader::SpawnedReader` owns a cancellation token: dropping it cancels the reader; a cancelled run resolves to `FileReaderError::Cancelled`.

### Struct fields added (exhaustive literals and patterns break)

- `config::InputSpec::Rtp` and `InputSpec::TsUdp` gain `socket: UdpSocketSpec` (flattened in JSON, so the JSON shape is additive). `UdpSocketSpec` gains `reuse_port: bool`.
- `push::RtspTransportConfig` gains `timeouts: rtsp_runtime::RtspTimeouts`; `push::RtmpTransportConfig` gains `connect_timeout` and `write_timeout` (both `Option<Duration>`; `None` means 15 s and 10 s). All keep `Default`, so use `..Default::default()`.
- `origin::HttpLimits` gains `queue_timeout: Duration`; `config::Config` gains `concurrency_queue_timeout_secs: f64` (serde default 5 s).
- With the `test-seams` feature enabled anywhere in your dependency graph, `Config` also gains `prebound`. Because `Config` is exhaustive, every `Config { .. }` literal then needs `..Default::default()`; prefer `Config::default()` plus assignment.

```rust
// before
RtmpTransportConfig { /* existing fields */ }
// after
RtmpTransportConfig { write_timeout: Some(Duration::from_secs(5)), ..Default::default() }
```

### Test hooks moved behind features

The WHIP/WHEP `*_for_test` entry points (`output::whep::{whep_router_for_test, serve_whep_for_test, ...}`, `source::whip::{serve_for_test, parse_offer_for_test, render_answer_for_test, whip_router_for_test, ...}`, `WHEP_TEST_OFFER`) are no longer in the default API; enable the non-default `test-hooks` feature to get them back. The pre-bound-socket seams (`Config::prebound`, `PreboundBinds`) sit behind the dependency-free `test-seams` feature, which `test-hooks` implies.

### Other API movement

- `ProgramSegmenter` and `push::{rtmp, rtsp, srt}` are now `pub`.
- `DvrRecorder` never appends into a period file written by an earlier recorder; `IndexEntry::seq` and the numbers served at `catchup/seg-{n}` are now the playlist numbers (Trunk number plus media-sequence offset). They equal the old values until a source reconnects or the process restarts; an archive written by an older version keeps its own numbers.
- `Backoff`'s public shape (`new`, `production_default`, `next`, `delay_for_attempt`, `reset`) is unchanged, but its delays are now jittered (below).
- Locks are `parking_lot`; `src/lock.rs` is deleted. A panicking holder can no longer poison a lock. The DVR fail-closed property is kept: a panic while persisting a `DvrRecorder` stops recording for that program and increments `multimux_dvr_failed_total`.

## Behaviour changes (config and wire)

### Newly rejected configuration

A config that loaded on 0.10 fails validation (also on admin add/reload) if it uses any of:

- a route `name` that is not a single safe path segment: characters outside `[A-Za-z0-9._-]`, `.`/`..`, a trailing `.` or space, a Windows reserved device name, longer than `MAX_ROUTE_NAME_LEN` (255), or two names differing only in case;
- two outputs mounting the same manifest path (push, `custom` and `whep` outputs mount no path, so several of those on one route stay valid);
- an `rtsp_push` URL whose scheme is not `rtsp://`/`rtsps://`, or a hostless one (`rtsp:cam`);
- an SRT push URL with a missing host, a `mode` other than `caller`, a `passphrase`, an out-of-range `latency`, an unbracketed IPv6 authority, or userinfo. `streamid` and `latency` query parameters are now applied to the handshake instead of being part of the dial address;
- a `ReconnectPolicy` with a zero backoff, `initial_backoff_ms` above `max_backoff_ms`, or a backoff over 24 h; an `ingest_connect_timeout_secs`/`ingest_read_timeout_secs` outside `(0, 86400]` or non-finite; a non-finite, non-positive or over-24 h `target_duration_secs` (exactly 86400.0 is accepted); a part longer than one segment;
- a `TsUdp` route whose pre-bound socket carries a `multicast_group`; DVR `period_duration_secs: 0` with only count-based retention.

`config::validate_host_port` is as strict as before despite now going through `url` (`host:9000/path`, `user@host:9000` and `host:9000?x` are still rejected).

### HTTP serving

- Listeners are served by a hyper-util `http1` server: 10 s header-read timeout, connection cap 1024, graceful drain up to 30 s, **no total per-connection deadline** (long LL-HLS/DVR/TS responses are not truncated). Plain HTTP/1 only: **h2c prior-knowledge is no longer served.**
- axum 0.8 path syntax is `{param}`/`{*rest}`; relevant to `Custom` outputs that build routes.
- The concurrency bound is one server-wide pool (`origin::limit`), with a smaller separate pool for LL-HLS blocking reloads and `/healthz`, `/readyz`, `/metrics` exempt. A request that cannot get a permit within `concurrency_queue_timeout_secs` (default 5) gets `503` with `Retry-After` and counts in `multimux_http_shed_total{kind}`.
- CORS: preflight now allows `Authorization` (it was `*`, which the Fetch spec does not extend to `Authorization`, so authenticated cross-origin browser players failed). Header names render lowercased and the origin adds `Access-Control-Expose-Headers`. WHIP/WHEP preflight answers `Access-Control-Allow-Methods: POST, PATCH, DELETE, OPTIONS`.
- `Cache-Control: immutable` is sent only on names that carry the origin's instance token (`init-{t}-{instance}-{gen}.mp4`, `seg-{t}-{instance}-{msn}.*`, `part-{t}-{instance}-{msn}.{i}.*`); token-less segment names get `max-age=10`; every non-success response is now `no-cache`. A route restart renames all of these and, without DVR, renumbers from 1. The bare `init-{t}.mp4` and token-less names still resolve. Any CDN rule keyed on the old names must be revisited.
- `.ts` segments of a `ts_hls` route are served as `video/mp2t` (they were `video/mp4`).
- `master.m3u8` carries the measured peak `BANDWIDTH` and a `CODECS` value instead of a fixed 5 Mb/s.
- The catch-up playlist omits `EXT-X-PLAYLIST-TYPE` whenever a window or DVR retention can remove segments (RFC 8216 §6.2.1/§6.2.2) and declares `EXT-X-MEDIA-SEQUENCE:0` when empty; each archived run gets its own `EXT-X-MAP` served at `catchup/init-p{N}.mp4`.

### WHIP and WHEP

- `POST` is answered only on `/whip` and `/whep`; `PATCH`/`DELETE` on `/whip/session` and `/whep/session` answer `405` with `Allow` (the `201 Location` resource is still not deletable, as before). Chunked request bodies are accepted; the hand-rolled request reader is deleted. Signalling bodies are capped at 64 KiB (413, rejected before any body byte is read), a whole request must complete within 10 s, each listener allows 256 in-flight signalling requests (a request-concurrency pool; the TCP connection cap is the listener-wide 1024), and capacity is checked before a UDP socket is bound (503 when full). The earlier draft's 16 KiB header cap (431) and 411-on-chunked refusal belonged to the deleted reader and are not claimed here.
- WHEP applies output auth (401 plus challenge; OPTIONS exempt), ends a session after 30 s without RTP, RTCP or a completed DTLS handshake, and rejects an offer without `a=fingerprint`. Any SRTCP packet that authenticates counts as liveness, so browser viewers sending PLI/NACK/REMB are no longer dropped after 30 s (needs `MediaEvent::RtcpUnsupported` from webrtc-runtime 0.2.0).
- SDP answers are built with `sdp-types` `Session::write`. The `o=`/`c=` lines carry the actual local IP instead of `127.0.0.1`; the WHIP answer's `m=` format list names only the chosen payload type (`m=video 9 UDP/TLS/RTP/SAVPF 96`, not `96 97 98`); WHIP offer candidates are read from the media section only; ICE credentials resolve media-level first. Answer order is pinned by `tests/golden/whip_answer.golden` and `whep_answer.golden`.

### Output formatting

- **DASH `xs:duration` is balanced by `jiff`.** `@minimumUpdatePeriod` and `@timeShiftBufferDepth` of 60 s or more change spelling: `PT60S` becomes `PT1M`, `PT3600S` becomes `PT1H`. Values under 60 s are byte-identical, and a fractional target such as 0.1 s over 3 segments now prints `PT0.3S` instead of `PT0.30000000000000004S`. Both spellings are valid `xs:duration`, but a string-matching test or monitor on the old text breaks. The same rule is shared with transmux's DASH writer, see [transmux-0.25.0.md](transmux-0.25.0.md).
- **Smooth Streaming client manifest changed shape:** HEVC tracks are no longer advertised (they 404ed), each `StreamIndex` has its own `c` timeline in absolute 10 MHz ticks, `QualityLevel@Bitrate` is a real bitrate (it was the ordinal) and so is the `QualityLevels(N)` in fragment URLs, `mfhd.sequence_number` is the segment number. Fragment requests are now answered with the requested track's own samples (previously every `StreamIndex` got the same muxed segment), and a codec Smooth cannot describe is omitted rather than advertised as `FourCC="H264"`. The manifest bytes of an unchanged stream are otherwise identical, now written through `quick_xml::Writer`.
- A live DASH `$Number$` route starts at the live edge (from `availabilityStartTime`, `Period@start`, `presentationTimeOffset` and a 3-segment presentation delay), not at `@startNumber`; a dynamic `$Time$` MPD plans only its last few timeline entries.
- Redaction of a parseable URL now goes through `url`, so host case, trailing `/` and percent-encoding are normalised in log text; an unparseable URL collapses to `<redacted>`.

### Reconnect and ingest

- Every retry delay is **equal-jittered**: the capped exponential series is multiplied by `uniform[0.5, 1.0)`, so a delay lies in `[raw/2, raw)` and stays spread at the cap (`backon`'s add-only jitter would collapse to exactly the cap). `supervisor::Backoff`, `ReconnectPolicy::backoff_for`, the push reconnect engine, the file reader's probe retry and HLS-pull resource retry share `reconnect::ReconnectSchedule`. Tests or alerting that assumed exact `min * factor^n` delays must change.
- A route rejects a second concurrent RTMP/WHIP publisher; once the first session ends, a new one for that program is accepted. A startup log line warns for each ingest route running with no authentication.
- Digest challenges from output and admin auth use `Verifier::challenge_for`, so an expired nonce yields `stale=true` (RFC 7616 §3.3). Digest acceptance is also stricter in broadcast-auth 0.4.0.
- UDP inputs bind through `socket2`: new optional keys `recv_buffer_bytes`, `reuse_address`, `multicast_interface`, `reuse_port` (Unix only) as siblings of the input object, and a 4 MiB `SO_RCVBUF` is now requested by default (best effort; the OS may clamp it).
- The pull sources share one scheduler: an HLS `WaitMs` hint is a pacing floor on the next playlist fetch, and a ready segment fetch is no longer held behind it.
- The RTSP, RTP, TS-UDP, TS-HTTP and SRT inputs share one ingest scaffold with bounded writes; the RTSP source's outbound write has a 10 s bound, and `HandshakeTimedOut` ends the route promptly.
- RTSP push uses `rtsp-runtime`'s `AsyncRtspClient` and RTMP push `rtmp-runtime`'s `AsyncRtmpClient`; every awaited IO is bounded. RTSP push now targets the configured URL (it used a hard-coded `rtsp://localhost/push`), checks every response status, writes the 401 `AuthRetry` bytes, sends RTP-framed MP2T, and sends a best-effort `TEARDOWN` on close. A `454 Session Not Found` surfaces as the new `RtspPushError::SessionLost`, so the push reconnects. The ANNOUNCE SDP bytes are pinned by `tests/golden/rtsp_announce.sdp`.

## Added

- **DASH SCTE-35 inband events (#969):** the MPD declares `<InbandEventStream schemeIdUri="urn:scte:scte35:2013:bin">` and served fMP4 segments carry `emsg` boxes (after `styp`, before `moof`) for segments with resolved SCTE-35 events. This was inert in production until this release: nothing recorded segment starts, so every segment saw no events (#1083). Id-less events (`time_signal`) now get unique ids instead of all colliding on `0`.
- `multimux::redact::{redact_url, redact_destination}`, `multimux::reconnect`, `source::udp::{bind_udp, UdpBindOptions}`, `origin::limit`, `output::whep::run_whep`, `RouteHandle::{release_program, with_archive_floor, await_trunk_change}`, `source::release_route`, `DvrRecorder::{needs_poll, with_seq_offset}`, `DvrConfig::retention_active`, `RtpUdpIngestSession::depay_dropped`, `source::rtsp::DEFAULT_KEEPALIVE_INTERVAL`, `DashIngestSession::with_clock`, `ReconnectPolicy::validate`, `config::MAX_ROUTE_NAME_LEN`.
- Pre-bound listener entry points for embedders and tests: `origin::{serve_with_registry_on, serve_with_registry_on_admin, serve_config_file_with_registry_on_admin, PreboundListeners}`, `WhepRoute::with_listener`, `WhipRoute::with_listener`, `TsUdpRoute::with_socket`.
- Metrics: `multimux_http_shed_total`, `multimux_pull_fragment_abandoned_total`, `multimux_pull_stream_refresh_miss_total`, `multimux_dvr_pin_rearmed_total`, `multimux_dvr_failed_total`, `multimux_dvr_si_errors_total`. `multimux_route_up` now reports `1` when Live and `0` after a route is removed (it never reported `1` before).

## Fixes (user-visible defects)

Ingest and egress
- A source reconnect no longer silences push and WHEP outputs (they stayed on the first `Trunk`); they are rebound to the new one. The HLS media sequence continues across a reconnect (#1089).
- RTMP and WHIP accept a publisher that connects after the route has been up longer than the connect timeout (each session's handshake clock starts at its own admission).
- A Data/Subtitle track appearing mid-stream no longer makes an fMP4 route publish MPEG-TS bytes as `seg-*.m4s`; a changed `avcC`/ASC/SPS now reaches the init segment.
- A non-finite `target_duration_secs` or `NaN`/`1e20` ingest timeout no longer panics the route (#1083).
- RTSP: a wrong password is classified as an auth failure instead of retrying forever; a periodic `OPTIONS` keepalive (clamped to `[KEEPALIVE_MIN, KEEPALIVE_MAX]`) keeps sessions alive on servers enforcing `Session;timeout=N`; a session that answers keepalives but sends no media is detected as stalled; a DESCRIBE `404` is retried 5 times before failing the route.
- Pull sources: a live `$Number$` MPD produced no samples; a tolerated `404` could stall a DASH or Smooth route forever; Smooth refresh matched streams by `Type` alone; one failed HLS resource fetch ended the whole session; Smooth "encrypted" detection scanned `mdat` bytes and falsely tore down a clear stream roughly every 45 minutes. WHIP inbound RTP header extensions are kept (#1090).
- A mid-segment PTS discontinuity no longer retro-stamps the segment being filled.

DVR, catch-up and admin
- DVR: a partial write left offsets short and served corrupt bytes for the rest of the period; the archive is now adopted across reconnect and restart rather than orphaned; byte retention counts the open period; the index sidecar is extended in place (about 25 ms and 680 KB per segment before) and fsynced properly.
- A `Stall`-policy DVR pin that the non-blocking safety valve force-expires no longer stops recording for good (it re-arms and counts a gap). `ProgramSegmenter` no longer blocks a runtime worker on a stalled DVR pin (media-plane #1082).
- Catch-up scans, index reads, archive reads, the DVR persist and the file-input probe run on the blocking pool; a handful of unauthenticated `catchup.m3u8` requests could previously stall ingest on every route. `catchup.m3u8?window_secs=N` is correct across a reconnect.
- The runtime admin API could leak runtimes under concurrent add/reload and never started WHEP outputs; `add_route`, `reload` and `remove_route` now serialise, and WHEP outputs are served.
- The metrics middleware no longer buffers response bodies, so an in-progress LL-DASH segment streams while produced (#721).

## Known limitation

The WHIP/WHEP `201 Created` `Location` resource is still not deletable (`DELETE` answers `405`; RFC 9725 §4.2 expects it deletable). Same as 0.10.0, tracked as a follow-up.

## Upgrading

Take this release with the sibling releases listed at the top; `Cargo.lock` must resolve them together. Outside multimux, code that builds `webrtc_runtime::media::MediaTransportConfig` itself needs the new `remote_fingerprint` field (see the webrtc-runtime 0.2.0 note). MSRV 1.95.0.

---

Published from tag `multimux-v0.11.0`.
