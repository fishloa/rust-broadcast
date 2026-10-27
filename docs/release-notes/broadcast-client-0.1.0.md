---
status: DRAFT
---
# broadcast-client 0.1.0

_Released 2026-10-24._

Initial release. `broadcast-client` is a generic **sans-IO client session for live and on-demand media**. You give it a URL; it tells you which HTTP requests to make or which TCP connection to open, you feed back what arrived, and it hands you demuxed, join-annotated samples plus every decision a player needs (when to start, pause and resume, when credentials are needed, what was dropped and why). It owns auth, redirects, cookies, retry/backoff, stall handling, demuxing and buffering policy for four sources behind one API: TS-over-HTTP (`tshttp-runtime`), HLS (`hls-runtime`), DASH (`dash-runtime`) and RTSP (`rtsp-runtime`). It performs no I/O, reads no clock, spawns no thread. Time is a caller-supplied monotonic `now_us`. The requirements come from the zenith-tv player, which is the first consumer; the policy is implementation-defined (no standard specifies it) and is documented in `docs/broadcast-client/DESIGN.md`.

## Quick start

```rust
use broadcast_client::{Config, Session};

let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
while let Some(action) = session.poll(0) {
    // host performs I/O based on `action`
    let _ = action;
}
```

This is the README snippet (it is a doctest, run by `cargo test --doc -p broadcast-client`). The runnable `examples/live_loop.rs` replays a committed TS capture through a session and `examples/auth_wait.rs` shows the credentials round trip; both run with no arguments.

## Sources

`Config::kind` selects the source. `SourceKind::Auto` picks RTSP for `rtsp://`/`rtsps://`, HLS for a `.m3u8`/`.m3u` URL path, DASH for a `.mpd` URL path (all ASCII case-insensitive) and TS-over-HTTP otherwise.

| Source | Backed by | What it does |
|---|---|---|
| TS-over-HTTP | `tshttp-runtime` | Live or finite (`Content-Length`) MPEG-TS over HTTP; reconnect with 2/4/8 s then 10 s backoff, start and stall timers, 404/410 reporting, one announced track set per connection |
| HLS | `hls-runtime`, `broadcast-hls` | Media and master playlists, variant choice, separate audio renditions, TS and fMP4 segments, VOD and live (blocking reload), byte ranges, `AES-128` decryption |
| DASH | `dash-runtime` | Static (VOD) and dynamic (live) MPDs: `SegmentTemplate`, `SegmentTimeline`, `SegmentList`, `SegmentBase` + `sidx`, multiple Periods, fMP4/CMAF segments, video and audio switching, a clock ladder for live (`UTCTiming`, `Session::set_wall_clock`, the response `Date`), conditional MPD refresh, early/expired/missing segment handling |
| RTSP | `rtsp-runtime` | SAT>IP tuners and IP cameras: DESCRIBE, SETUP per media, PLAY, RTP/RTCP interleaved over the TCP connection, H.264 + AAC |

## Feature table

| Area | Provided |
|---|---|
| Auth | Basic, Digest and Bearer through `broadcast-auth`; `Event::AuthRequired` then silence until `set_credentials`; `AuthRejected`; `Forbidden`; credentials in URL userinfo; credentials never leave the session origin |
| Redirects | `Config::max_redirects`; `http`/`https` (or `rtsp`/`rtsps`) only, never downgraded |
| Cookies | Host-only jar for the session origin (`Path`, `Secure`, `Max-Age`, `__Secure-`/`__Host-`), seedable and persistable (`initial_cookies`, `Session::cookies`) |
| Headers | `Config::extra_headers`, validated at open |
| Tracks | One `Event::Tracks` with the whole resolved set per TS connection (hold bounded by `tracks_hold_*`); merged set for HLS audio renditions |
| Join | Per-sample `JoinDecision` (`JoinPoint`/`Drop`/`Keep`) so a mid-GOP join starts at a random-access point |
| Buffering | Live TS by `buffering-runtime`; HLS and finite TS by a deterministic playout gate; `Event::Buffer(Start|Pause|Resume)` |
| Timeline jumps | Detected without a discontinuity flag (`Config::timeline_jump`), emitted as `Event::Discontinuity` |
| Serde | Optional `serde` feature |

## Host contract

- Drive the session with `poll(now)` until it returns `None`, and drain `next_event()` after EVERY callback (`on_response`, `on_data`, `on_closed`, `on_connected`, `on_playback_position`, `set_credentials`, `reconnect`): actions and events are produced inside the callbacks.
- Actions: `HttpRequest` (`byte_range` is `(offset, length)`), `TcpConnect` (the host does TLS for `rtsps`), `Send`, `Close`, `WakeAt(t)` ("poll at or after `t`"; a host that polls only at `WakeAt` times sees every timer).
- Live TS decides `Event::Buffer` inside `on_playback_position(now, pos)`: call it every display tick, or no `Start` is produced.
- `Session::close()` is terminal; callbacks for a connection the session already closed are ignored.
- After a fatal `Event::Error` the session closes every connection and emits nothing further.
- Output is independent of how the host chunks the bytes: the sample sequence and the position of every `Discontinuity` are identical for every read size (`tests/r6_chunk_invariance.rs`).

## Behaviour guarantees

- **Never silent.** Every drop or failure is a typed `Event::Error { kind, fatal }` (or a `Status`/auth event): `UnsupportedEncryption`, `KeyFetchFailed`, `InitFetchFailed`, `InitDecodeFailed`, `SegmentFetchFailed`, `BadKey`, `DecryptFailed`, `UnsupportedCodec`, `TrackIdReserved`, `TrackDropped`, `UnsupportedContainer`, `BodyTooLarge`, `UnknownRenderedTrack`. Non-fatal errors are reported once per kind until a success resets them; fatal ones mean the session will not recover and the player should show a message.
- **A bad body is a failed fetch, never a consumed segment.** An HTML error page, random bytes, an empty body or a fragment without its `mdat` served with `200` is retried; after `max_segment_failures` (8) a VOD playlist ends with one fatal `SegmentFetchFailed`, a live one reports it, emits `Discontinuity` and plays on.
- **Bounded memory.** Playlist, segment and key bodies are capped (`max_manifest_bytes`, `max_segment_bytes`, `max_key_bytes`); the event queue is capped (`max_queued_events`) and what it drops is counted (`dropped_events`), never silently lost.
- **Invalid configuration is rejected at open** (`Error::InvalidConfig`) when a value would silently disable output (zero timeouts, caps or jump thresholds).
- **Secrets stay out of logs.** `Action`'s `Debug` omits header values and `Send` payloads; cookies are never in `Debug`.
- **Encrypted media is never half-decoded.** `AES-128` HLS is decrypted; SAMPLE-AES, FairPlay/Widevine key formats and CENC/CBCS are reported as `UnsupportedEncryption` and never decrypted.

## Config knobs and defaults

`Config::default()` is the shipped, tested set: `max_manifest_bytes` 4 MiB, `max_segment_bytes` 64 MiB, `request_timeout_us` 20 s, `segment_timeout_us` 60 s, `hls_key_timeout_us` 20 s, `backoff_ms` `[2000, 4000, 8000]` with cap 10 s, `max_inflight_segments` 4, `max_key_bytes` 4096, `max_key_failures` 8, `max_segment_failures` 8, `max_queued_events` 100 000, `tracks_hold_max_input_bytes` 4 MiB, `tracks_hold_quiet_input_bytes` 512 KiB, `tracks_hold_max_media_us` 1 s, `tracks_hold_max_wall_us` 1 s, `start_pending_track_wait_us` 1.5 s, timeline-jump thresholds 1 s backward / 5 s forward slack, playout leads 1 s (VOD) and 6 s (live HLS, capped at three target durations). The live-TS buffer defaults come from `buffering-runtime` (1.8 s initial target).

## Validation

- Public tests (run everywhere): the in-tree suites for the three sources, auth, redirects, cookies, HLS keys, renditions, playout, timeline jumps, the start gate and chunk invariance, plus the lexical no-hand-roll guard, the label-coverage drift guard and the `#[non_exhaustive]` coverage guard.
- DASH: a fault matrix of 45 fault kinds (transport and status faults, `UTCTiming`, MPD refresh, Period churn, `sidx`, `425`/`Retry-After`, container and host-clock faults) over six request classes and eight presentation shapes, transient and permanent, each cell asserting the fault was applied, with a guard that fails when a fault or request class has no row; the fuzz target `dash_session`; `tests/no_clock_guard.rs`; and `tests/e2e_livesim2.rs`, 30 s of live DASH from DASH-IF livesim2 (pinned v1.15.2) over real HTTP in six shapes (it skips loudly without livesim2 and fails under `REQUIRE_LIVESIM2=1`, which the `dash-e2e` CI job sets). The matrix found two defects in `dash-runtime`, both fixed before release: a Period listed twice made the client replay its segments, and a segment that stayed `425` past its window was retried for the rest of the session.
- Real captures held in the private fixtures repository (CI checks them out with a read-only deploy key; a missing private fixture fails there): the start-gate and hold tests replay real head-end TS captures at real time.

## Known limits

- **RTSP carries H.264 and AAC only.** H.265 over RTP is not supported (`transmux` has no RFC 7798 depayloader); an SDP whose only media is H.265 fails with one fatal `UnsupportedCodec`. UDP RTP is not supported.
- **SRT, Smooth Streaming and RTMP pull sources are not included.** DASH plays fMP4/CMAF only, at segment granularity (a low-latency MPD plays whole segments and reports `UnsupportedFeature`), with no seeking and no automatic bitrate adaptation; encrypted, WebM, MPEG-2 TS and subtitle sets are dropped with a typed error.
- **Late audio on live TS.** `Start` waits for a PMT-declared audio or video stream that has no track yet, but only for `start_pending_track_wait_us` (1.5 s) after the first media. Audio that resolves later than that, or a start lead shorter than the audio skew, still produces `Start` then `Pause`; raise the wait or the start lead if your muxes skew audio that far.
- **`Retry-After` in HTTP-date form** is counted from the `set_wall_clock` anchor, else from the response's `Date`, and ignored when neither exists; the seconds form is honoured. Both are clamped to 60 s.
- HLS subtitle renditions are listed but not fetched; an HLS audio rendition is a separate client, aligned with video by the player on PTS.

## Dependencies

`broadcast-common` 10.0, `broadcast-auth` 0.5, `transmux` 0.28, `dvb-si` 12.0, `mpeg-ts` 0.6, `buffering-runtime` 0.1, `tshttp-runtime` 0.1, `hls-runtime` 0.10, `dash-runtime` 0.1, `broadcast-hls` 0.5, `rtsp-runtime` 0.8 (resolved to the versions of this release wave), plus `sdp-types`, `url`, `percent-encoding`, `http`, `cookie`, `thiserror` and optional `serde`. The crate is `std`; MSRV 1.95.0.

## Migration

New crate; nothing to migrate. `broadcast-client = "0.1"`.

---

Published from tag `broadcast-client-v0.1.0`.
