# broadcast-client: generic sans-IO client session (issue #1455)

Requirements: zenith-tv design spec 2026-10-23 (owner-approved option b: a Rust crate, no C facade;
the player owns the C interface). The host does only transport I/O (HTTP requests, later TCP/UDP);
this crate owns every protocol, auth, redirect, reconnect and buffering decision.

## Story split

1. **BC-1 (phase 1):** crate, `Session` API, auth + redirect layer, TS-over-HTTP source on `tshttp-runtime`
   (#1451), events Tracks/Sample/DisplayChanged/Status/Auth*. No buffering events yet.
2. **BC-2:** `Event::Buffer(Start|Pause|Resume)` from `buffering-runtime` (needs a playback-position input).
3. **BC-3 (phase 3):** HLS source on `hls-runtime::HlsClient` (master playlists, renditions) via the same
   Action/Event vocabulary. Out of scope for BC-1/BC-2.

## Crate

`broadcast-client`, `std` (broadcast-auth is std), deps: `tshttp-runtime`, `broadcast-auth`, `transmux`
(TrackSpec/Sample/DisplayInfo types are re-exported), `url`, later `hls-runtime`, `buffering-runtime`.
Must `cargo check` on aarch64-apple-tvos/-tvos-sim/-ios/-ios-sim/-darwin (#1454); no tokio, no reqwest, no
threads, no wall clock. Time = caller monotonic `u64` microseconds (`now_us`).

## API

```
Session::open(url: &str, config: Config, now_us: u64) -> Result<Session>     // no I/O
Session::poll(&mut self, now_us) -> Option<Action>       // advances timers, then pops
Session::next_event(&mut self) -> Option<Event>
Session::on_response(conn: ConnId, status: u16, headers: &[(String,String)])
Session::on_data(conn: ConnId, bytes: &[u8], now_us)
Session::on_closed(conn: ConnId, error: Option<TransportError>, now_us)
Session::set_credentials(Credentials)                    // broadcast_auth::Credentials
Session::reconnect(now_us)                               // caller-initiated (decoder failure)
Session::close()                                         // terminal; Close for open conns, no more events but Status(Ended)? no: Closed
```
`ConnId` is `u32` allocated by the session. `now_us` is caller monotonic microseconds.

`Config { kind: SourceKind(Auto|TsHttp), ts_http: tshttp_runtime::Config, max_redirects: 5,
extra_headers: vec![] (reserved for #1458) }`. `Auto` for an `http(s)` URL = TsHttp in BC-1 (HLS added in BC-3
by extension `.m3u8`). Other schemes: `Error::UnsupportedScheme`.

```
Action (non_exhaustive): HttpRequest{conn, method: "GET", url, headers: Vec<(String,String)>, byte_range: Option<(u64,u64)>}
                       | Close{conn}            // drop connection, ignore later callbacks for it
                       | WakeAt(u64)            // call poll again at/after this now_us
                       // reserved, never emitted in BC-1: TcpConnect, UdpBind, Send
Event  (non_exhaustive): Tracks(Vec<TrackSpec>) | Sample{track: TrackId, sample: Sample}
                       | DisplayChanged{track, pts: Option<i64>, display: Arc<DisplayInfo>}
                       | Discontinuity | Finite{expected_bytes} | Status(Status) | Starved(StarveKind)
                       | AuthRequired{scheme, realm, origin, resource_metadata: Option<Url>}
                       | AuthRejected{status} | Forbidden{status}
Status: Establishing | Live | Stalled{secs} | Reconnecting{attempt} | NotFound | WaitingForCredentials | Ended
```

## Behaviour (each line gets a named test)

**Event mapping from tshttp-runtime (BC-1):**
- `TrackAdded/TrackUpdated/TrackRemoved` maintain a spec map; `Event::Tracks(full set)` is emitted once per
  change, immediately before the next `Sample` or `DisplayChanged` (coalesced: N updates between two samples
  give one Tracks). The first Sample of a connection is always preceded by Tracks.
- `Sample`/`DisplayChanged` pass through in order (DisplayChanged keeps its position between samples).
- `Connected` -> `Status(Establishing)` stays until the first Sample, then `Status(Live)` once.
- `StallNotice{secs}` -> `Status(Stalled{secs})`; the next received byte emits `Status(Live)` again.
- `Reconnecting{attempt}` -> `Status(Reconnecting{attempt})`; a played-then-ended reconnect (0 ms) emits
  `Status(Reconnecting{attempt:0})`. A new connection starts a fresh demuxer: `Discontinuity` is emitted
  before the first Sample of every connection after the first.
- `NotFound{..}` -> `Status(NotFound)` (re-emitted per 404/410, retries continue at the cap).
- tshttp `Event::Finite{expected_bytes}` -> `Event::Finite{expected_bytes}`.
- tshttp `Event::Ended` -> `Status(Ended)`; the session does not reconnect after Ended; `Session::reconnect()`
  restarts (attempt 0).
- `Starved(kind)` -> `Event::Starved(kind)`.
- `StreamingTsDemux` events other than Track*/Sample/DisplayChanged are dropped.
- tshttp `Action::Open` -> `HttpRequest`; `Abort` -> `Close{conn}` of the live conn; `Reconnect{after_ms}` ->
  `WakeAt(now+after_ms)`; the client-owned timer then re-opens through `poll(now)`.

**Auth layer (wraps every HttpRequest):**
- Credentials are bound to the ORIGIN (scheme+host+port) of the session URL, never to anything else. A request
  to any other origin carries no Authorization.
- Bearer: `Authorization: Bearer <token>` preemptively on every request to the origin once set.
- Basic/Digest (`Credentials::new(user, pass)`): first request unauthenticated; on 401 parse
  `WWW-Authenticate` with `broadcast_auth::parse_challenges`, build `Authenticator::from_challenge`, replay the
  SAME request once with `Authorization`; later requests (including after reconnect) send it preemptively,
  Digest nc/nonce state kept in the Authenticator; a 401 with `stale=true` re-challenges transparently.
- 401/407 with no credentials set: `Event::AuthRequired{scheme (first challenge, preferring Bearer when the
  challenge carries resource_metadata), realm, origin, resource_metadata}`, `Status(WaitingForCredentials)`,
  `Action::Close`, NO retry and NO timers until `set_credentials` or `close`.
- 401 after credentials were sent: `Event::AuthRejected{status}`, `Status(WaitingForCredentials)`, same wait.
- `set_credentials` while waiting: reopens immediately (attempt counter reset). While streaming: stored and used
  for the next request; an in-flight stream is not interrupted (Bearer refresh without restart).
- 403: `Event::Forbidden{status}` then handled as a failed connect by the retry policy.
- Credentials are never logged, never in `Debug`, never stored beyond the session.

**Redirects (the host MUST NOT follow redirects itself):** 301/302/303/307/308 with `Location`: resolve against
the request URL (`url::Url::join`), issue a new HttpRequest (+Close old conn), at most `max_redirects` (5) per
open, then fail as a connect error. A redirect to a different origin DROPS credentials for that request
(the original-origin credentials stay bound to the original origin). 3xx without Location = connect error.

**Status codes** reach tshttp-runtime only after this layer: 2xx -> `on_response(2xx)`; 404/410 and other
non-2xx -> `on_response(status)` (the layer handles 401/407/3xx itself).

**Errors/robustness:** unknown ConnId callbacks are ignored (late callbacks after Close). `on_closed(error)` maps
to tshttp `on_error`, `None` to `on_eof`. `close()` is terminal: emits `Close` for open conns once, then
`poll`/`next_event` return None.

## Tests required (BC-1)

One named test per bullet above using the real fixtures: TF1 capture `private/fixtures/zenith-http/tf1-live.ts`
(+ headers files) through `test_fixtures`, driven through a tiny in-test host (feeds on_response/on_data/on_closed
from the capture). Auth tests use constructed response headers from RFC 9110/6750/9728 verbatim examples
(see broadcast-auth/docs/www-authenticate-challenges.md) and Digest vectors already in broadcast-auth tests.
Chunking invariance (1-byte, 1316-byte, 64 KiB) of the Sample/Tracks sequence; Tracks-before-first-sample; Live
status once; stall -> Stalled -> Live; reconnect after server close emits Discontinuity; 401->replay->200 for
Basic and Digest; Bearer preemptive; 401 with no credentials waits (no actions for 10 minutes of poll calls);
AuthRejected; set_credentials resumes; cross-origin redirect drops Authorization; redirect limit; unknown conn
ignored; close terminal; Debug does not contain the secret; Apple-target `cargo check` in CI.

## BC-2: Buffer events (buffering-runtime inside the session)

Reference for the feeding rules (read-only, authoritative for "match what ships today"):
`/Volumes/External/Projects/zenith-tv/ZenithMedia/Sources/ZenithMedia/PlayerController.swift`
(`reportMediaEnds` ~l.955, `videoSample` join handling ~l.735-762, `pump` ~l.960, chunk feed ~l.551,
`restart` ~l.715) and `LiveBuffer.swift`. The soak baseline was tuned with these rules.

New API on `Session`:
- `Event::Buffer(BufferEvent)`, `BufferEvent::{Start{at_27mhz}, Pause{hold_at_27mhz, starving_track: Option<u32>}, Resume{at_27mhz}}`
  mapped 1:1 from `buffering_runtime::Event::{StartPlaying,Pause,Resume}` (media_27mhz/hold_at_27mhz/track kept; TargetChanged/OverMax/Rebaselined/EventsDropped are not surfaced).
- `Session::on_playback_position(now_us, Option<u64 /*pts 27 MHz*/>)` -> `BufferController::on_tick`.
- `Session::set_rendered_tracks(&[TrackId])` (player input: which tracks it renders; default = the first video and the first audio track announced). Registered with the controller via `register_track` (video track key = its TrackId.0, one controller track per rendered TrackId). Changing the set calls `register_track`/`unregister_track` for the difference.
- `Session::hint(Hint)` passthrough for the player-owned hints: InputDegraded, InputRecovered, SyncLoss, PcrDiscontinuity.
- `Config::buffer: buffering_runtime::ControllerConfig` (default = the crate default) and `Session::open_with_memory(.., Option<StallMemorySnapshot>)`/`Session::stall_memory() -> StallMemorySnapshot` so memory survives sessions (zenith seeds it today).

Automatic feeding inside the session (the host does nothing):
- `on_chunk(arrival_us, bytes, pcr_27mhz)` once per `on_data` call, arrival = the `now_us` passed to `on_data`, pcr = the LAST `DemuxEvent::ClockReference` ticks (27 MHz) observed during that chunk's demux, `None` if none (taken once: cleared after use).
- `on_track_media(now_us, track, media_end_27mhz)`: once per delivered chunk and rendered track, using the LAST sample of that track demuxed from the chunk: end = (pts + duration) converted to 27 MHz (`pts * 27_000_000 / timescale`). Video samples count only from the JOIN POINT: the first video sample for which the session's per-video-track `transmux::LiveJoin` (`LiveJoin::for_config(&spec.config)`, `push(&sample) -> JoinDecision`, `rejoin()` on timeline restart; `Sample` itself carries no join) returns `JoinDecision::JoinPoint(_)`; video samples before it (`Drop(_)`) are not reported; at the join point report ONE zero-length media end (end = that sample's pts) for the video track BEFORE any other report (the controller takes the first reported position as its start origin). Audio ends are withheld until the video join point exists (when a video track is rendered); a stream with no rendered video reports audio immediately.
- Hints: `Reconnected` BEFORE feeding the first chunk of every connection after the first; `SourceEnded` on EOF; `SourceError` on a transport error; `PcrDiscontinuity` when the demuxer emits `DemuxEvent::Discontinuity` (timeline restart) AND the join-point/withholding state resets (new timeline = wait for a new join point). The controller is NOT reset on a plain reconnect (it keeps its stall memory).
- Time unit: the controller takes microseconds; the session clock is already `now_us`.
- VOD (a `Finite` resource): the controller is not created (no live-edge decisions); `Event::Buffer` is never emitted; pacing is the player's media-clock business.

Tests (BC-2) compare the session against a bare `BufferController` oracle fed the same inputs by the test, and use the real TF1 capture (`zenith-http/tf1-live.ts`, 1316-byte chunks, synthetic arrival schedule steady 100 ms/chunk, then a 3 s gap, then steady).

`Event::Sample` gains `join: Option<transmux::JoinDecision>` (Some for video samples of a track that has a `LiveJoin`, None otherwise), so the player drops `Drop(_)` samples itself exactly as it does today; the session still emits them (it never drops samples). Reported as a breaking field addition to `Event::Sample` (BC-1 is not yet consumed by any player).

### BC-2 corrections from the player's baseline (zenith-tv review; binding)

- (a) Video media end is NOT the last sample's pts+duration: it is `maxPTS + duration of the current (latest kept) sample`, where `maxPTS` = the max PTS (presentation order) over KEPT video samples seen so far since the join point (B-frames: the last sample in decode order can have a lower PTS). Reference: PlayerController.swift l.759-762 (`lastVideoPTS = max(lastVideoPTS, s.pts)`; `pendingVideoEnd = videoTime(lastVideoPTS + s.duration)`). Only samples from the join point on count; `Drop(_)` and pre-join samples never move it. Reported once per chunk (the latest pending value).
- (b) Audio media end = max over the RENDERED audio track of (pts + duration), reported only once it is past the video join point's PTS (`end > join_pts`, l.916). When the rendered audio track changes (set_rendered_tracks with a different audio), forget the old track's end and (re)register the new one. Audio samples of non-rendered tracks never count.
- (c) `Session::rejoin()`: decoder reset / player restart WITHOUT a new timeline: re-arms every video track's `LiveJoin` (`rejoin()`), clears pending ends and maxPTS/join state, does NOT hint `PcrDiscontinuity` and does NOT reset the controller. On a new timeline the player calls `rejoin()` AND `hint(PcrDiscontinuity)`. The session's own automatic PcrDiscontinuity (demuxer `Discontinuity`) does both.
