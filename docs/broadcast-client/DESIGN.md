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
Session::on_response(conn: ConnId, status: u16, headers: &[(String,String)], now_us)
Session::on_data(conn: ConnId, bytes: &[u8], now_us)
Session::on_closed(conn: ConnId, error: Option<TransportError>, now_us)
Session::set_credentials(Credentials)                    // broadcast_auth::Credentials
Session::reconnect(now_us)                               // caller-initiated (decoder failure)
Session::close()                                         // terminal; one Close per live conn (RTSP: a Send carrying TEARDOWN first when a session was set up), no more events
```
`ConnId` is `u32` allocated by the session. `now_us` is caller monotonic microseconds.

`Config { kind: SourceKind(Auto|TsHttp), ts_http: tshttp_runtime::Config, max_redirects: 5,
extra_headers: vec![], initial_cookies: vec![] (#1458) }`. `Auto` for an `http(s)` URL = TsHttp in BC-1 (HLS added in BC-3
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
- tshttp `Action::Open` -> `HttpRequest`; `Abort` -> `Close{conn}` of EVERY conn the session knows (pending
  and live); `Reconnect{after_ms}` -> `WakeAt(now+after_ms)` (saturating); the client-owned timer then
  re-opens through `poll(now)`. After every drain the session also emits `WakeAt(next_deadline)` from
  tshttp's `next_deadline()` (gated on not waiting for credentials), so a host that only wakes on `WakeAt`
  sees the start timeout, stall notices and the stall reconnect.

**Auth layer (wraps every HttpRequest):**
- Credentials are bound to the ORIGIN (scheme+host+port) of the session URL, never to anything else. A request
  to any other origin carries no Authorization.
- Bearer: `Authorization: Bearer <token>` preemptively on every request to the origin once set.
- Basic/Digest (`Credentials::new(user, pass)`): first request unauthenticated; on 401 parse
  `WWW-Authenticate` with `broadcast_auth::parse_challenges`, build `Authenticator::from_challenge`, replay the
  SAME request once with `Authorization`; later requests (including after reconnect) send it preemptively,
  Digest nc/nonce state kept in the Authenticator; a 401 with `stale=true` re-challenges transparently, but
  at most ONCE per request lineage (TS, HLS and RTSP): a second stale answer is `AuthRejected` and the wait.
- 401/407 with no credentials set: `Event::AuthRequired{scheme (first challenge, preferring Bearer when the
  challenge carries resource_metadata), realm, origin, resource_metadata}`, `Status(WaitingForCredentials)`,
  `Action::Close`, NO retry and NO timers until `set_credentials` or `close`. `resource_metadata` is
  attacker-controlled: the server that answered the challenge chose that URL (any `https` origin). A host that
  fetches it (RFC 9728) MUST validate it first (scheme `https`, an expected or allow-listed host, no private,
  loopback or link-local address) or it is an SSRF vector; the session itself never fetches it.
- A 407 is answered with `Proxy-Authorization` (from the same credentials) and its challenge is read from
  `Proxy-Authenticate`; a 401 is answered with `Authorization` from `WWW-Authenticate`. Once a 407 was seen
  the preemptive header on later requests to the origin is `Proxy-Authorization` until a 401 is seen.
- When several challenges are offered (several header lines or several challenges in one line) the
  authenticator is built from all of them and the strongest wins, Digest over Basic (RFC 9110 11.6.1);
  Bearer credentials answer with `Bearer` whatever was offered.
- 401 after credentials were sent: `Event::AuthRejected{status}`, `Status(WaitingForCredentials)`, same wait.
- `set_credentials` while waiting: reopens immediately (attempt counter reset). While streaming: stored and used
  for the next request; an in-flight stream is not interrupted (Bearer refresh without restart).
- 403: `Event::Forbidden{status}` then handled as a failed connect by the retry policy.
- Credentials are never logged, never in `Debug`, never stored beyond the session.

**Redirects (the host MUST NOT follow redirects itself):** 301/302/303/307/308 with `Location`: resolve against
the request URL (`url::Url::join`), issue a new HttpRequest (+Close old conn), at most `max_redirects` (5) per
open, then fail as a connect error. A redirect to a different origin DROPS credentials for that request
(the original-origin credentials stay bound to the original origin). 3xx without Location = connect error.
Only `http` and `https` targets are followed and an `https` request is never redirected to `http`; anything
else is a connect error. A 401/407 from an origin other than the session origin is a connect error too (Close
plus backoff, no credential prompt, no replay); a replay lineage that carries no `Authorization` is capped at
two replays. Every final non-2xx answer closes and forgets its connection. `user:password@` in an `http(s)://`
or `rtsp(s)://` URL is percent-decoded into initial Basic/Digest credentials (a later `set_credentials` wins)
and stripped from every request URL.

**Status codes** reach tshttp-runtime only after this layer: 2xx -> `on_response(2xx)`; 404/410 and other
non-2xx -> `on_response(status)` (the layer handles 401/407/3xx itself).

**Errors/robustness:** unknown ConnId callbacks are ignored (late callbacks after Close). `on_closed(error)` maps
to tshttp `on_error`, `None` to `on_eof`. `close()` is terminal: it drops every queued action except
`Close`, adds one `Close` per live conn (never twice for the same conn), then `poll`/`next_event` return
None. For RTSP `close()` first queues one `Send` carrying `TEARDOWN` (only when a session id exists), then the
`Close`; every other source emits `Close` only.

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
- `Session::set_rendered_tracks(&[TrackId])` (player input: which tracks it renders; default = the LOWEST track id of each kind among the announced tracks, see "BC-2 rendered-set history"). Registered with the controller via `register_track` (video track key = its TrackId.0, one controller track per rendered TrackId). Changing the set calls `register_track`/`unregister_track` for the difference.
- `Session::hint(Hint)` passthrough for the player-owned hints: InputDegraded, InputRecovered, SyncLoss, PcrDiscontinuity.
- `Config::buffer: buffering_runtime::ControllerConfig` (default = the crate default) and `Session::open_with_memory(.., Option<StallMemorySnapshot>)`/`Session::stall_memory() -> StallMemorySnapshot` so memory survives sessions (zenith seeds it today). `BufferEvent::Pause` carries `reason: PauseReason` (`NoData`, `DataArriving`, `Discontinuity`, `Reprime`) mapped from `buffering_runtime::Reason`; `Session::buffer_stats() -> BufferStats` exposes `stalls`, `reprimes`, `discontinuities`, `longest_freeze_us`.

A 206 response to our own plain GET is finite too. Redirect/auth layers are unchanged; headers are
forwarded to tshttp-runtime for finite detection. `Event::Finite` is mapped from tshttp, and
`Event::Ended` becomes `Status(Ended)`. The session does not reconnect after Ended; `Session::reconnect()`
restarts the session (attempt 0) - this is how a host loops a finite TS file. `Status::Ended` of an HLS session
(ENDLIST reached) is terminal instead: afterwards `reconnect()`, `select_variant()` (a known index is accepted,
an unknown one is `Error::NoSuchVariant`) and `select_audio_rendition()` are no-ops that queue no action and emit
no event; a live HLS session never ended is unaffected. Truncated transfers (EOF before all expected bytes) reconnect.

### BC-2 corrections from the player's baseline (zenith-tv review; binding)

- (a) Video media end is NOT the last sample's pts+duration: it is `maxPTS + duration of the current (latest kept) sample`, where `maxPTS` = the max PTS (presentation order) over KEPT video samples seen so far since the join point (B-frames: the last sample in decode order can have a lower PTS). Reference: PlayerController.swift l.759-762 (`lastVideoPTS = max(lastVideoPTS, s.pts)`; `pendingVideoEnd = videoTime(lastVideoPTS + s.duration)`). Only samples from the join point on count; `Drop(_)` and pre-join samples never move it. Reported once per chunk (the latest pending value).
- (b) Audio media end = max over the RENDERED audio track of (pts + duration), reported only once it is past the video join point's PTS (`end > join_pts`, l.916). When the rendered audio track changes (set_rendered_tracks with a different audio), forget the old track's end and (re)register the new one. Audio samples of non-rendered tracks never count.
- (c) `Session::rejoin()`: decoder reset / player restart WITHOUT a new timeline: re-arms every video track's `LiveJoin` (`rejoin()`), clears pending ends and maxPTS/join state, does NOT hint `PcrDiscontinuity` and does NOT reset the controller. On a new timeline the player calls `rejoin()` AND `hint(PcrDiscontinuity)`. The session's own automatic PcrDiscontinuity (demuxer `Discontinuity`) does both.

### BC-2 rendered-set history (late `set_rendered_tracks`)

The session runs ahead of the player: it emits samples before the host can call `set_rendered_tracks`. For every announced track (rendered or not) the feeder keeps bounded progress metadata, no sample bytes: first pts, video join point (pts, timescale), max kept video pts and the latest end (`max_pts + duration`), audio max (`pts + duration`). When a track enters the rendered set (explicit declaration or a changed default) the controller is seeded from it right after `register_track`: `video` gets the zero-length join report then its end; audio is reported only when its end is past the rendered video's join PTS (rule (b) unchanged). The seed is flushed at once with the last chunk time, because a registered track without an end blinds the controller (`effective()` is `None`, ticks do nothing) until its next delivery, and after the last chunk there is none. A track that never emitted has no history and is waited for exactly as before. Removing a track from the rendered set unregisters it but keeps its history, so re-declaring it is also seamless. `rejoin()`, a demuxer/timeline discontinuity and a track leaving `Event::Tracks` drop the history (the video `LiveJoin` re-arms with it).

Default set: `Session::default_rendered_tracks() -> (Option<TrackId>, Option<TrackId>)` = the lowest `TrackId` of the video kind and of the audio kind among the announced tracks (not first-added: track ids follow resolution order, a host assumes lowest-id). Until `set_rendered_tracks` is called the controller follows it, re-targeting (unregister old, register + seed new) when a lower id is announced. `Event::Tracks` is a tuple variant, so the default is exposed by the getter only. The BC-5 playout gate (HLS, finite TS) uses the same rule (`buffer::lowest_id_per_kind`); its per-track observations already cover non-rendered tracks, so a late declaration needs no seeding there.

## BC-3: HLS source (hls-runtime::HlsClient inside the session)

Scope split: **BC-3a** single media playlist and master playlist with the policy-chosen variant (muxed audio), TS and fMP4 segments, VOD and live. **BC-3b** separate audio renditions as selectable tracks (built, see "BC-3b renditions"; subtitle renditions remain later). No Event::Buffer for HLS in BC-3a; HLS and finite TS get the BC-5 playout gate (below).

- `SourceKind::Hls`; `Auto` picks Hls when the URL path ends with `.m3u8`/`.m3u` (ASCII case-insensitive), else TsHttp.
- Adapter `source/hls.rs` over `hls_runtime::client::HlsClient` (default features, no tokio):
  - `Action::FetchPlaylist{..}` -> `HttpRequest` GET to `action.playlist_request_url()` (blocking-reload query appended by hls-runtime), conn tagged `Playlist`. `Action::FetchResource{id,url,byte_range}` -> `HttpRequest{.., byte_range}` tagged `Resource(id)`. The session keeps a conn table `ConnId -> purpose`. In-flight segment and part requests are capped at `Config::max_inflight_segments` (default 4, `0` means 1) per playlist adapter: the cap counts every segment/part that was issued and not yet delivered to the core (in flight, or completed and held for in-order delivery, or failed and awaiting retry), so held bodies are bounded by `cap * max_segment_bytes`; the rest of hls-runtime's actions stay queued and are released as responses complete, delivery to the core stays in order. A retry of a segment already inside the window is never blocked by the cap (a failed segment cannot deadlock the window); playlist, key and init requests are not counted and never wait behind segments (queued actions leave in the order the core emitted them among those allowed to go). Request timeouts are tracked in a deadline heap with lazy deletion, so `poll` is O(log n) per expiry, not a scan of every request.
  - Bodies accumulate per conn across `on_data` (bounded: `Config::max_manifest_bytes` 4 MiB, `max_segment_bytes` 64 MiB; exceeding = Close + failure) and are delivered on `on_closed(None)`: `on_playlist(bytes)` / `on_resource(id, bytes)`. A `HlsClient` parse error is a failure of that request.
  - `Action::WaitMs(ms)` -> `WakeAt(now_us + ms*1000)`; `HlsClient::next_wait()` converts the same way (the core takes `std::time::Instant` only in `poll_timeout`; the session never calls that, it computes deadlines from `now_us`).
  - Output mapping: `Master(sel)` -> `Event::Variants(MasterSelection)` (new event, variants+chosen+renditions as in hls-runtime); `Tracks` -> `Event::Tracks`; `Init` dropped (not surfaced); `Samples{track_id, samples}` -> one `Event::Sample{track, sample, join}` per sample, in order (`join` set by the session, see Audit round 5 F5); `DisplayChanged` -> `Event::DisplayChanged`; `Discontinuity` -> `Event::Discontinuity`; `EndOfStream` -> `Status::Ended`.
  - Status: `Establishing` until the first Sample, then `Live` once; `Status::Ended` for ENDLIST VOD at end of stream.
- New inputs: `Config::variant_policy: hls_runtime::client::VariantPolicy`; `Session::select_variant(index)`; both delegate to HlsClient.
- Retry policy (the core has none, "the caller drives retry cadence"): a failed request (transport error, non-2xx other than 401/407/3xx handled by the auth/redirect layers, truncated or oversized body, parse error) calls `HlsClient::on_error(id)` and schedules the next `poll` retry with the same table as TS-HTTP: backoff 2, 4, 8 s then 10 s forever (`Config::backoff_ms`, `backoff_cap_ms`), the attempt counter of playlist, key and init failures resets on a successful playlist; a failing SEGMENT has its own per-resource counter (see Round 8) that grows the same schedule and resets only when that segment is delivered as media. 404/410 on the MAIN (first) playlist: `Status::NotFound` per occurrence + same backoff; 404/410 on a segment: counted as a failure for that resource only (no NotFound status). While in backoff `Status::Reconnecting{attempt}` is emitted on each scheduled retry. A playlist reload is never given up on (a playlist cannot be skipped); a segment is, after `Config::max_segment_failures` consecutive failures (VOD: one fatal `SegmentFetchFailed`; live: one non-fatal `SegmentFetchFailed` then `Event::Discontinuity`, playback continues). The terminal states are `Ended` (ENDLIST), a fatal error and `close()`. Every `backoff_ms` entry and `backoff_cap_ms` is raised to at least 1 ms, so a zero never produces a burst of retries at one instant. A `429`/`503` with a `Retry-After` (seconds or HTTP-date, clamped to 60 s) stretches the wait to at least that long via the shared helper, and emits one `Event::ServerRefused { status, retry_after_us, reason }` per refused attempt when its connection ends (the refused connection is closed at the response unless its body can become the reason, i.e. it is `text/plain` UTF-8 and not declared empty; then it is closed as soon as the body is complete (`Content-Length` reached or `on_closed`) or `MAX_SERVER_REFUSED_REASON_BYTES` are captured; a body with neither a length nor an `on_closed` is closed at the next request or `close()`), where `reason` is the response body only when it is `text/plain` (UTF-8 or no charset), control characters stripped, trimmed and capped at `MAX_SERVER_REFUSED_REASON_BYTES`.
- Auth/redirect layers apply to EVERY request exactly as for TS-HTTP: credentials scoped to the session origin (playlist origin) only; segments on other origins (CDN) carry no Authorization; per-request redirects (each request has its own redirect counter).
- Stall handling: no bytes for `Config::ts_http.stall_*` does NOT apply; a request in flight longer than `Config::request_timeout_us` (default 20 s, playlists) / `segment_timeout_us` (default 60 s) is aborted (`Close`) and counts as a failure.

Tests (BC-3a): named test per bullet, using the real fixtures (fixtures/hls/real/bipbop-ts master + `hls-runtime/tests/fixtures/ts-hls` classic TS segments; cmaf-fmp4 fixture for fMP4; master with the policy; VOD ENDLIST => Ended once; backoff table; 404 main playlist vs segment; auth scoping on a cross-origin segment; per-request redirect; bounded body; request timeout; blocking-reload query in the HttpRequest URL; prefetch concurrency table), plus a chunking-invariance test (1-byte, 1316, 64 KiB body chunks give the identical Event sequence) and an oracle test (the session's Sample sequence equals driving a bare HlsClient with the same bodies).

Automatic feeding inside the session (the host does nothing):
- `on_chunk(arrival_us, bytes, pcr_27mhz)` once per `on_data` call, arrival = the `now_us` passed to `on_data`, pcr = the LAST `DemuxEvent::ClockReference` ticks (27 MHz) observed during that chunk's demux, `None` if none (taken once: cleared after use).
- `on_track_media(now_us, track, media_end_27mhz)`: once per delivered chunk and rendered track, using the LAST sample of that track demuxed from the chunk: end = (pts + duration) converted to 27 MHz (`pts * 27_000_000 / timescale`). Video samples count only from the JOIN POINT: the first video sample for which the session's per-video-track `transmux::LiveJoin` (`LiveJoin::for_config(&spec.config)`, `push(&sample) -> JoinDecision`, `rejoin()` on timeline restart; `Sample` itself carries no join) returns `JoinDecision::JoinPoint(_)`; video samples before it (`Drop(_)`) are not reported; at the join point report ONE zero-length media end (end = that sample's pts) for the video track BEFORE any other report (the controller takes the first reported position as its start origin). Audio ends are withheld until the video join point exists (when a video track is rendered); a stream with no rendered video reports audio immediately.
- Hints: `Reconnected` BEFORE feeding the first chunk of every connection after the first; `SourceEnded` on EOF; `SourceError` on a transport error; `PcrDiscontinuity` when the demuxer emits `DemuxEvent::Discontinuity` (timeline restart) AND the join-point/withholding state resets (new timeline = wait for a new join point). The controller is NOT reset on a plain reconnect (it keeps its stall memory).
- Time unit: the controller takes microseconds; the session clock is already `now_us`.
- VOD (a `Finite` resource): the controller is not created (no live-edge decisions); the BC-5 playout gate emits the `Event::Buffer`s instead (see "BC-5 playout gate").

Tests (BC-2) compare the session against a bare `BufferController` oracle fed the same inputs by the test, and use the real TF1 capture (`zenith-http/tf1-live.ts`, 1316-byte chunks, synthetic arrival schedule steady 100 ms/chunk, then a 3 s gap, then steady).

`Event::Sample` gains `join: Option<transmux::JoinDecision>` (Some for video samples of a track that has a `LiveJoin`, None otherwise), so the player drops `Drop(_)` samples itself exactly as it does today; the session still emits them (it never drops samples). Reported as a breaking field addition to `Event::Sample` (BC-1 is not yet consumed by any player).

### BC-3a implementation notes (as built)

- Core actions are drained into a session-side queue immediately after every `HlsClient` call. A `WaitMs` gates every action behind it until `now_us` reaches the deadline (`WakeAt`); `HlsClient::next_wait` is not needed because the queue is inspected directly.
- Resources are fetched concurrently but delivered to `HlsClient::on_resource` in issue order (`Init` immediately): a later segment that completes first is held until every earlier one is delivered, so `Sample` order is independent of completion order. A failed resource blocks later ones until it is re-requested (or the playlist reload no longer asks for it).
- Retry: every failure bumps one attempt counter, emits `Status::Reconnecting{attempt}`, emits `WakeAt(now + backoff)` and gates the core queue until then. A failed playlist is refetched (same URL) at the wake time; a failed live resource relies on the next playlist reload (the live reload chain, or a refetch scheduled when the chain is dead) to be re-requested after `HlsClient::on_error`; a failed resource of a VOD playlist (`EXT-X-ENDLIST`) is re-requested at the wake time by `HlsClient::retry_resource` and the playlist is NOT refetched. The playlist/key/init counter resets when a playlist is accepted by the core; a segment's counter does not (Round 8).
- Every HTTP request also emits `WakeAt(now + timeout)` so a host that only wakes on `WakeAt` still fires the timeout.
- A 401/407 from a host other than the session origin is an ordinary failed request (never answered with credentials).
- hls-runtime accepts unrecognised segment bytes without error (no samples, stream still ends); only playlist parse/UTF-8 errors, demux errors on corrupt init-bound data, and duplicate/unrequested deliveries surface as `Err` and become failures.
- hls-runtime fix made with BC-3a: `EndOfStream` now also waits until every segment of the last playlist is delivered (a failed segment under ENDLIST no longer ends the stream early). Known remaining limitation: after an `on_resource` error, `on_error(Some(id))` decrements the outstanding-fetch counter a second time.

## BC-3b renditions

A master playlist can reference a separate audio rendition group (`EXT-X-MEDIA TYPE=AUDIO` with `URI`, referenced by the variant's `AUDIO="group-id"`). The session plays it with a SECOND `HlsClient` adapter (`source/hls.rs`, role `Audio`) living in `source/hls_session.rs` next to the main one. Subtitle renditions are not handled.

- One `HlsSession` owns both adapters and merges them. Both share the session's `AuthState` (credentials, cookies, `extra_headers`, origin scoping), the redirect/retry/key machinery of the adapter, and ONE connection-id allocator (`ConnAlloc`, an atomic counter), so a `ConnId` is unique across adapters. Callbacks are routed to the adapter whose request table contains the conn; a conn that belongs to no adapter (closed, torn down) is ignored.
- Selection (`Config::auto_select_audio`, default true; `Config::preferred_audio_language`): when `Event::Variants` arrives, the candidates are the renditions of the CHOSEN variant's `AUDIO` group with audio type, in master order. Pick: first candidate whose `LANGUAGE` matches the preferred language (BCP 47 prefix, ASCII case-insensitive: `de` matches `de` and `de-DE`) and has `AUTOSELECT` or `DEFAULT`; else the first `DEFAULT=YES`; else the first candidate with a URL. A picked rendition WITHOUT `URI` is muxed in the variant: it is reported active but nothing is fetched. A rendition whose URL equals the variant playlist URL never starts a second adapter. A variant without an audio group never starts one.
- `Session::select_audio_rendition(Option<usize>)`: the index addresses the candidates above for the CURRENT chosen variant. `None` stops separate audio. Errors: `NotHls` (non-HLS session), `NoSuchRendition` (out of range, or no master seen yet). A manual call replaces automatic selection for the rest of the session. Switching closes every connection of the old audio adapter, drops it (late callbacks are ignored) and starts the new one at its live edge. A failing rendition never reaches the video adapter.
- `select_variant(i)` updates the chosen variant and re-evaluates audio: automatic mode re-runs the pick, a previous `None` stays none, a manual rendition is kept by `(NAME, LANGUAGE)` if the new group has it (otherwise the automatic pick). An unchanged target URL keeps the running adapter.
- Events: audio tracks use `track id + AUDIO_TRACK_ID_OFFSET` (1000), so the main playlist's track ids must stay below 1000; `Event::Tracks` is always the MERGED set (main + remapped audio) and is re-emitted whenever either side changes or the audio adapter is dropped; audio samples are `Event::Sample{track: remapped, join: None}`; `Event::Variants` is unchanged; `Event::AudioRendition{index, name, language}` is emitted whenever the active rendition changes (`index: None` when none, including a muxed rendition reported as active). `Discontinuity` and `DisplayChanged` of either adapter are forwarded.
- Status: `Establishing` and `Live` once per session (Live precedes the first sample of either adapter); `Ended` only when the main adapter reached ENDLIST AND the audio adapter (if any) did too - a permanently failing audio rendition therefore blocks `Ended` until the player calls `select_audio_rendition(None)`; `NotFound` is emitted only for the main playlist (an audio playlist 404 is an ordinary failed request); `Reconnecting{attempt}` comes from either adapter, each with its OWN attempt counter and backoff gate.
- Fatal errors (`Event::Error{fatal: true}`) from either adapter stop the whole session: both adapters are halted and `select_audio_rendition`/`select_variant` start nothing.
- Known limitations: an audio rendition is a separate `HlsClient`, so audio and video are not synchronised at the transport level (the player aligns them on PTS); the audio adapter treats its URL as a media playlist (a master there yields no audio).

Tests: `broadcast-client/tests/hls_renditions.rs`. The separate audio-only and video-only segments are derived in the test by demuxing the real `ts-hls` fixture and re-muxing each track with `transmux`'s `StreamingTsHlsSegmenter`; the oracle is a bare `HlsClient` per stream, and the real bipbop master covers the URI-less default rendition.

## BC cookies/headers (#1458)

Binding spec: `broadcast-client/docs/cookies.md` (RFC 6265 verbatim plus the first-slice decisions). Summary of what is built:

- `src/cookies.rs`: jar scoped to the session origin; domain-match and path-match/default-path are `cookie_store`'s pure `CookieDomain`/`CookiePath` (a `Domain` attribute is accepted only if the origin host domain-matches it, otherwise the cookie is refused, and an accepted one is still sent ONLY to the session origin - origin scope is a deliberate security property, since no public-suffix data is shipped; no public-suffix list; no `cookie_store` type that reads the wall clock is used); `Expires` ignored; `Secure` only on https; `Max-Age` relative to the `now_us` of the response (<= 0 deletes, expiry is exclusive: gone at `stored_at + max_age`); same name+path replaces in place; `Cookie` header ordered longest path first, then storage order; at most 50 cookies (oldest evicted) and 4096 bytes of name+value per cookie. `Set-Cookie` lines are parsed with the `cookie` crate; every `Set-Cookie` line of any status (401 and 3xx included) is processed, but only for responses of the session origin.
- The jar lives in `AuthState`, the single layer every request passes through (`request_headers(url, now_us)`), so the TS-HTTP and HLS adapters share it: playlists, segments, reconnects and in-origin redirects carry the cookies; requests to another origin carry no cookie (whatever its `Domain`), no extra header and no `Authorization`.
- `Config::initial_cookies` seed the jar (path `/`); `Session::cookies()` exports the live `(name, value)` pairs for the host to persist. Values are secrets: `Debug` of `Config`, `Session`, `AuthState` never prints them.
- `Config::extra_headers` are validated in `Session::open` with `http::HeaderName`/`HeaderValue` (`Error::InvalidHeader`), reserved names `Authorization`, `Cookie`, `Host`, `Content-Length`, `Transfer-Encoding`, `Connection`, `Range` are `Error::ReservedHeader`. Order sent: extra headers in the given order, then `Cookie`, then `Authorization`. `initial_cookies` with an invalid name/value are `Error::InvalidHeader`.
- HLS key requests (`EXT-X-KEY`, #1459 S2) go through the same `request_headers` path (see the key section below).

## BC-3 HLS AES-128 keys (#1459 S2)

Binding spec: `broadcast-hls/docs/ext-x-key.md`. The core (`hls-runtime`) emits `Action::FetchKey{id,url}` once per distinct key URI and decrypts AES-128 segments itself; the session only moves bytes.

- A key is a third request purpose (`Purpose::Key(KeyId)`) through the same path as playlists and segments: origin-scoped `Authorization`, cookies and `extra_headers` (a key on another origin carries none of them), per-request redirect budget, a 401/407 from the session origin enters the normal `AuthRequired`/wait flow and the parked request is replayed after `set_credentials`.
- `Config::hls_key_timeout_us` (20 s) and `Config::max_key_bytes` (4096; a larger body is closed and counts as a failure).
- A complete 200 body goes to `HlsClient::on_key`. A non-2xx, transport error, timeout, oversize or `on_key` error calls `HlsClient::on_key_error` and uses the shared retry schedule: `Status::Reconnecting`, backoff 2/4/8 s then 10 s, and the retry is the playlist refetch that makes the core re-request the key (`Status::NotFound` is never emitted for a key). While key/decrypt failures are outstanding a successful playlist does not reset the attempt counter (otherwise every retry would wait 2 s); decoded samples reset it, and a key accepted by the core resets it.
- `Event::Error { kind: ErrorKind, fatal }` tells the player why nothing plays. Kinds: `UnsupportedEncryption{method}` (SAMPLE-AES and other non-AES-128 methods, non-identity `KEYFORMAT` such as FairPlay/Widevine - `method` then holds the KEYFORMAT -, encrypted `EXT-X-MAP`, I-frames-only, a key without `URI`), `KeyFetchFailed{status}`, `BadKey` (not 16 bytes), `DecryptFailed` (wrong key / bad padding), `UnsupportedCodec{fourcc}` (defined, not emitted by BC-3). Labels via `ErrorKind::name()` are stable snake_case.
- Fatal policy: `UnsupportedEncryption` is fatal at once. `KeyFetchFailed`, `BadKey` and `DecryptFailed` are non-fatal: emitted once per kind until decoded samples reset it, the session keeps retrying; after `Config::max_key_failures` (8) consecutive such failures with no samples in between one `fatal: true` Error of the same kind is emitted. 401/407/403 on a key request are never `Error`s: they stay `AuthRequired`/`AuthRejected`/`Forbidden`.
- `fatal` is terminal: the source emits the Error, issues `Close` for every open connection, forgets queued work and timers, and then ignores every callback and `Session::reconnect`/`select_variant`/`set_credentials`; `poll` returns nothing and no further events are produced.
- Reconnect (`Session::reconnect`) keeps the core's key cache (keys are cached per URI for the life of the session, `Expires` ignored): the post-reconnect playlist does not refetch a key that was already fetched; an in-flight key request at reconnect time is reported to the core as failed and re-requested by the refetched playlist.
- Resources still reach the core in issue order; a segment that arrives before its key is held by the core and decrypted when the key lands.
- Known core limitation: a key rejected by decryption stays cached in the core, so after `DecryptFailed` the retries re-fetch the segment but never replace the key; the session therefore ends in the fatal `DecryptFailed` after `max_key_failures`.

## BC-4 RTSP (#1460)

RTSP client source for SAT>IP tuners and IP cameras, built on `rtsp-runtime`'s sans-IO `ClientSession`
(request builders, CSeq/Session bookkeeping, `$`-frame decoding, transparent 401 retry) and
`transmux`'s `RtpStreamDepacketiser`. The logic is ported from `multimux`'s `RtspIngestSession`
(phases, keepalive rule) without depending on `multimux`.

- `SourceKind::Rtsp`; `Auto` picks it for `rtsp://` and `rtsps://`. A mismatching scheme/kind pair is
  `Error::UnsupportedScheme`; an RTSP URL without host is `Error::InvalidUrl`.
- Transport: RTP over the RTSP TCP connection only (`RTP/AVP/TCP;interleaved`). UDP RTP is out of scope;
  a SETUP reply that is not TCP-interleaved is a failure. The ports 554 (`rtsp`) and 322 (`rtsps`) are
  the IANA defaults, defined in the adapter because `rtsp_runtime`'s constants sit behind its `tokio`
  feature.
- New actions: `Action::TcpConnect { conn, host, port, tls }` (IPv6 hosts without brackets; `tls` is true
  for `rtsps`: the HOST performs TLS and certificate validation, the session only sees plaintext) and
  `Action::Send { conn, bytes }`; `Close { conn }` is unchanged. New host input `Session::on_connected(conn)`;
  a failed connect is the existing `on_closed(conn, Some(error), now)` before connected. `on_response` is
  not used for RTSP. Unknown or closed `conn` callbacks are ignored.
- Handshake: on connect DESCRIBE, then SETUP per SDP media (interleaved `0-1`, `2-3`, ...; the channel the
  server negotiates is the one used for routing), then PLAY. Track ids are the 1-based SDP media order; SDP
  parsing is `sdp-types`, fmtp/rtpmap decoding is `transmux::rtp_sdp`. Per-track SETUP URIs resolve a relative
  `a=control` against the base of RFC 2326 Appendix C.1.1: the DESCRIBE response's `Content-Base`, else its
  `Content-Location` (both through the additive `ClientSession::content_base()`), else the request URL with a
  trailing `/` guaranteed (`rtsp://cam/axis-media/media.amp` + `trackID=1` is `.../media.amp/trackID=1`);
  `a=control:*` is the base itself and an absent control is the request URL. A session-level `a=control` is the
  aggregate control URL (RFC 2326 C.1.1), resolved the same way against the base (a foreign-authority one is
  ignored): PLAY, the keepalive OPTIONS and TEARDOWN use it instead of the request URL when present. Of the
  payload types a media line lists, the first whose own `a=rtpmap` and `a=fmtp` (matched by payload type)
  describe H.264 or AAC is the one played (an H.264 payload type named by `a=rtpmap` with no `a=fmtp` is playable, its parameter sets being taken in-band, RFC 6184), so a `rtx` payload type listed first does not hide the real one. An absolute control whose
  scheme, host or port differs from the connection's skips that section. A media section the client cannot
  play (other than H.264 video and AAC audio, a missing or unusable fmtp) is skipped; an SDP with no playable
  section left, or unparsable SDP, is a failure. At most 16 sections are played (later ones are ignored).
  The channels the server negotiates (`ClientSession::take_negotiated_transport()`, so a SETUP reply without a
  `Transport` header negotiates nothing) must be TCP-interleaved and must not repeat or overlap the channels
  of an earlier track, else the connection fails with backoff.
- Media: interleaved frames are routed by channel: the track's RTP channel to `push`, RTP channel + 1 to the
  depayloader's `push_rtcp` (sender reports; no receiver reports are sent). `Event::Tracks` before the first
  `Sample` after PLAY of every connection, `Event::Sample { join: None }` per sample, `Status::Establishing`
  on connect, `Status::Live` once at the first sample. A malformed RTP frame is counted
  (`Session::rtp_malformed_count`) and skipped; `RtpLossEvent::SequenceGap`/`DamagedAccessUnit` are counted
  (`Session::rtp_loss_count`) and the stream continues (`NonMonotonicTimestamp` is B-frame reordering, not
  loss, and is not counted). When a connection ends after PLAY the depayloader is flushed so the last access
  units are delivered.
- Keepalive: `OPTIONS` on the aggregate control URL (else the request URI) at half the `Session` header's `timeout` capped at 3600 s (no floor: a floor could land at or after the server's expiry),
  default 30 s (also for `timeout=0`), armed at the PLAY reply and re-armed at each send, driven by `poll(now_us)`. Any reply to it
  is ignored (405/501 are common); a reply while playing resets the auth replay budget, so keepalive
  challenges with rotating Digest nonces never exhaust it. `close()` sends `TEARDOWN` (only when a session id exists) and then
  `Close { conn }`; the same TEARDOWN-then-Close is used when the session aborts a connection itself (stall
  reconnect, start timeout, handshake failure after SETUP, `reconnect()`).
- Auth: credentials live in the session's `AuthState` (origin = the RTSP URL) and are given to
  `ClientSession` (`authenticate_with`, an additive `rtsp-runtime` API that also exposes `last_challenge()`;
  `ClientEvent::Response` carries no headers, so the challenge could not be read otherwise). A 401/407 with
  no credentials: `Event::AuthRequired { scheme, realm, origin, resource_metadata: None }` +
  `Status::WaitingForCredentials`; the connection stays open and every timer is suspended (no wake, no
  request) until `set_credentials` or `close`; `set_credentials` re-sends the challenged request (already
  carrying `Authorization` built from the recorded challenge) and restarts the start timeout. Credentials
  that `ClientSession` already retried and the server still refuses: `Event::AuthRejected { status }` + the
  same wait. 403: `Event::Forbidden { status }` then the failed-connect policy. 404 on DESCRIBE:
  `Status::NotFound` then the same backoff. A new connection starts a new `ClientSession` seeded with the
  stored credentials and the last challenge.
- Reconnect/stall policy is the TS-over-HTTP table, applied from `Config::ts_http` (the same `Config` governs
  both): start timeout 15 s counted from connect to the first sample (restarted per connection, suspended
  while waiting for credentials); after the first sample, no interleaved RTP bytes for `stall_notice_ms` (5 s)
  gives `Starved(NoData)` once and `Status::Stalled { secs }` per second, the next RTP byte gives
  `Status::Live`, and `stall_reconnect_ms` (30 s) aborts and reconnects at once with the attempt counter reset;
  bytes without a sample for 5 s give `Starved(Data)` once; a connection that ends after it played reconnects at
  once (attempt reset, `Event::Discontinuity` when the next connection is established); a failure before the
  first sample (connect error, bad reply, start timeout, garbage, bad SDP) gives `Status::Reconnecting
  { attempt }` and the backoff `backoff_ms` (2, 4, 8 s) then `backoff_cap_ms` (10 s) forever. `tshttp_runtime`'s
  client demuxes MPEG-TS inside `feed`, so its timer code cannot be driven from RTP; the same transitions are
  implemented in `source/rtsp.rs` and pinned by `tests/rtsp.rs`. As for TS-HTTP, a played-then-ended reconnect
  emits no `Reconnecting` status.
- Deadlines: the adapter emits `WakeAt(deadline)` for the earliest of backoff due, start timeout, stall
  notice/reconnect and keepalive, only when no earlier wake is outstanding.
- Not in this slice: `Event::Buffer` for RTSP, cookies/`extra_headers`, userinfo credentials in the URL, RTCP
  receiver reports, UDP RTP, `Content-Base` handling, server-initiated requests, video other than H.264, audio
  other than AAC.

Tests: `broadcast-client/tests/rtsp.rs` with a fake RTSP server in `tests/support/rtsp_host.rs` (parses the
session's `Send` bytes with `rtsp-types`, answers with SDP built by `sdp-types`, pushes `$`-framed RTP made by
`transmux`'s packetiser from the real `fixtures/ts/h264_aac.ts` samples; the oracle is the original samples,
payload bytes and timestamps). Digest responses are verified by `broadcast-auth`'s own `Verifier`.

### RTSP track clocks

Each RTSP track's RTP timestamps start at an unrelated random base (RFC 3550 5.1), so the raw sample times of two tracks are not comparable. `source/rtsp_align.rs` (`Aligner`) puts every track on one common timeline before a sample leaves the RTSP source, so the buffer, the playout gate and the host all see aligned pts and dts.

- Anchor: the video track. Its pts/dts are never changed. Every other track gets one tick offset (`delta`, in its own timescale) added to pts and dts. Without a video track the first track to deliver is the anchor. Samples of other tracks wait (at most 256) until the anchor's first sample exists; if it never comes the earliest waiting track becomes the anchor.
- `AvSync::Provisional`: no Sender Report mapping for the track yet. `delta` lines the track's first sample up with the anchor's first sample, so the error is the arrival skew between those two samples.
- `AvSync::SenderReport`: the depacketiser has an SR (RFC 3550 6.4.1, NTP time to RTP time) for the anchor and the track. `RtpStreamDepacketiser::sync_start_decode_times` (the only place the NTP arithmetic lives) gives each track's first-sample wall-clock distance from the earliest one; the target `delta` places the track's first sample at the anchor's first pts plus its true wall-clock offset from the anchor's first sample. Every new SR recomputes the target.
- Switch: forward correction is applied immediately; backward correction is slewed by at most half a sample duration per sample, so per-track pts never go backwards. A correction larger than `SR_SWITCH_TOLERANCE_US` (100 ms, about five AAC frames, near the edge where audio/video offset is read as out of sync) is a real timeline change: `Event::Discontinuity` once, then the new `delta`. The provisional error is the arrival skew, normally a few packet times, so the common case is absorbed silently.
- Reporting: `Event::AvSync(AvSync)` is per session, emitted before the first sample (`Provisional`, or `SenderReport` when the SRs were already in) and once more when every non-anchor track is on SR timing. A single-track presentation never emits it. The state resets with the connection (a reconnect builds a new depacketiser and new RTP bases).
- The buffer no longer shifts tracks: the lead is right by construction. `tests/rtsp_clock_offset.rs` guards Start timing against random per-track RTP bases.

Limits: a track that never gets Sender Reports stays Provisional; the tracks' SR NTP clocks are assumed to share a wall clock (RFC 3550 6.4.1 requires it for lip sync).

## BC-5 playout gate

`Event::Buffer(Start|Pause|Resume)` for the sources the PDV controller does not serve: HLS live, HLS VOD (`EXT-X-ENDLIST` seen) and finite TS (`Event::Finite`). Module `src/playout.rs`; live TS-over-HTTP keeps the BC-2 `buffering-runtime` path unchanged (`Config::buffer`). The gate is deterministic and lead-based: it does not measure jitter, so its events depend only on the delivered samples, the end of stream and the reported playback position, never on chunking or wall-clock time.

Inputs (all from the session): per-rendered-track media end (27 MHz, the BC-2 definition: video end = max PTS of kept video samples since the origin + the latest sample's duration, audio end = max of pts + duration; finite TS and HLS skip `JoinDecision::Drop` samples), `on_playback_position(now_us, Option<pts_27mhz>)`, `set_rendered_tracks` (default: lowest id of each kind, see BC-2 rendered-set history), `rejoin()`, `Event::Discontinuity` (and a demuxer discontinuity), a variant switch, and end of stream (`Status::Ended`; for HLS only once the main and any audio rendition playlist ended). The gate sees merged track ids, so the BC-3b audio rendition (id + 1000) is an ordinary audio track in the minimum.

Rules, with `lead = min over rendered tracks of end - reference`:
- Origin = PTS of the first sample (since the last reset) of the first rendered video track, else of audio. A rendered video track reports only from its first sample.
- Before the start every rendered track must have reported; `Start { at_27mhz = origin }` when `min end - origin >= start lead`. If the stream ends first, `Start` is emitted at the end (using whatever has reported), so the player never waits forever.
- Once started and not paused: `Pause { hold_at_27mhz = position, starving_track = track with the smallest end }` when the position is `Some`, `min end - position <= pause_lead` and the stream has not ended. Position `None` is never evaluated; after the end it never pauses.
- While paused (reference = the held position): `Resume { at_27mhz = hold }` when `min end - hold >= resume lead`, or immediately when the stream ends. At most one transition per input.
- A reset (discontinuity, variant switch) clears the origin, ends and state, so `Start` fires again; `rejoin()` does the same but keeps the ended flag. Events of the gate precede `Status::Ended` when the end triggers them.

Config: `Config::playout: PlayoutConfig { vod_start_lead_us: 1_000_000, live_start_lead_us: 6_000_000, resume_lead_us: None (= the start lead of the source kind), pause_lead_us: 0 }`. For live HLS the start lead is `min(3 x EXT-X-TARGETDURATION, live_start_lead_us)`: `HlsClient::target_duration_secs()` and `endlist_seen()` were added to hls-runtime as additive accessors for this (and for the VOD/live decision). `Event::Buffer` types are unchanged; nothing is new for the player except `Config::playout`.

Behaviour changes versus BC-2/BC-3a: finite TS and HLS now emit Buffer events (BC-2 finite TS and BC-3a HLS emitted none); the controller log (`buffer_input_log`, test seam) stays empty for them.

Tests: `broadcast-client/tests/playout.rs` (HLS through the real HLS session with the classic-TS ts-hls and CMAF fixtures plus a 12 s contiguous stream built from the same fixture samples for live, finite TS from `fixtures/ts/h264_aac.ts` with `Content-Length`). The oracle in the test recomputes the leads from the delivered `Event::Sample`s independently of the gate.

## Pre-release audit fixes (#1484)

Behaviour pinned by `tests/audit_1484.rs`, `tests/audit_1484_sources.rs` and the in-crate unit tests.

- `Action` has a manual `Debug`: header NAMES only (never values), `Send` prints its length, never the bytes.
- Every failed connect (403, 3xx without `Location`, redirect over the limit) emits `Close{conn}`. A late
  response, data or close for a conn the session already closed is ignored and never becomes the live conn.
- TS-over-HTTP reconnects re-open the ORIGINAL URL (a redirect is followed again); a 401 replay targets the
  URL that answered 401.
- `Discontinuity` and `Hint::Reconnected` are emitted only for a connection that follows one that reached
  `Connected`; a first open answered 503 then 200 is not a discontinuity. A new connection resets the finite
  flag and disengages the finite-TS playout gate (`Event::Finite` re-arms both).
- A `Tracks` event without a previously known track makes the buffer feeder forget that track.
- `Session::set_credentials` returns `Result<(), Error>`: `Error::InvalidCredentials` for control characters
  (or a bearer token that is not a valid header value). Cookie names must be RFC 7230 tokens
  (`Error::InvalidHeader`).
- `select_audio_rendition` after `Status::Ended` is accepted and ignored (nothing is started, nothing is
  emitted). After a fatal `Event::Error` no later event of the same batch is forwarded.
- A cross-origin 401/407 on a key request counts as `KeyFetchFailed { status: Some(status) }` (reported once
  per kind, fatal at `max_key_failures`); 403 stays `Forbidden`.
- Arithmetic is saturating or checked: reconnect delays, `pts + duration`, the 27 MHz conversion (clamps, never
  truncates), connection ids (a wrapped counter skips ids still live; HLS ids are leased). A jump of the host
  clock stays bounded: the TS source catches up a jump over 60 s in 1 s steps (at most one stall-notice
  cadence per step), the RTSP source emits at most 64 stall notices per poll, RTSP keepalives stop at the end
  of the representable time, and a `WakeAt` already issued for the same instant is not repeated.
- RTSP bounds the `stale=true` retries `rtsp-runtime` performs on its own to two replays per lineage
  (`AuthRejected` afterwards).
- `Config::max_queued_events` (default 100 000) caps the whole event queue. When the host does not drain
  `next_event`, transient `Status` events and `Starved` notices coalesce to their latest, then the oldest
  `Sample`s are dropped (counted by `Session::dropped_events()`) and each burst of dropped samples leaves one
  `Event::Discontinuity` in their place so the player re-syncs. Tracks, errors, buffer events and terminal
  statuses are never dropped (the queue is bounded by the cap plus the undroppable events a stalled host leaves
  unread). It is a `VecDeque` of slots with per-kind lanes of sequence numbers: O(1) amortised, no scans. The action
  queue is not capped: dropping a `Send`, `HttpRequest` or `Close` would corrupt the protocol state. Every
  callback adds a bounded number of actions except the HLS fan-out, which is bounded explicitly: at most
  `Config::max_inflight_segments` segment/part `HttpRequest`s are outstanding per adapter (plus the
  playlist, key and init requests), however many segments the playlist lists.
- Cookies (RFC 6265bis): a `Secure` cookie from an `http` origin is rejected, `__Secure-` needs `Secure`,
  `__Host-` needs `Secure`, `Path=/` and no `Domain` (prefixes match ASCII case-insensitively), a `Path` over
  1024 bytes is rejected. Seeded cookies obey the same rules: a name over 4096 bytes (name plus value) is
  dropped, and `__Secure-`/`__Host-` cookies are accepted only on an `https` origin (stored `Secure`). The
  `Cookie` header never exceeds 16 KiB (cookies that do not fit are left out, longest path first).
- A byte-ranged HLS request (`EXT-X-BYTERANGE`) is answered with `206` and a `Content-Range` naming exactly
  that range, or with a `200` (server ignored `Range`) from which the range is sliced once the whole body
  arrived (the body cap applies to the whole file); anything else, or a body shorter than the range end, is a
  failed request.
- A main-playlist track id at or above `AUDIO_TRACK_ID_OFFSET` is dropped with its samples and reported once
  as `ErrorKind::TrackIdReserved`. When both HLS adapters report the one logical discontinuity at the same
  instant only the first reaches the playout gate and the host. A failed playlist delivery whose reload the
  engine already queued (unmergeable delta) adds no second reload chain.
- Playout: an effective resume lead is at least `pause_lead_us + 1 us`; a `set_rendered_tracks` made before the
  gate engages is applied when it does. 33-bit PTS/DTS and 32-bit RTP timestamps are unwrapped once at the
  demux edge (`transmux` `ts_demux`, `rtp_stream`), so every `Sample` timestamp the gate sees is an absolute
  unrolled `i64` and the gate needs no wrap logic.
- RTSP applies `Config::ts_http.clamped()` and the `min_reconnect_interval_ms` floor (only when the connection
  that played lasted less) like the TS source.
- `auth` and `redirect` are `#[doc(hidden)]` (the integration tests need them); everything else internal is
  crate-private.

## BC-6 timeline-jump detection

The player used to detect a new timeline itself (`PlayerController.isNewTimeline`) when the PTS jumped without a TS discontinuity flag (server restart, a looping recorder, a channel swap behind one URL). The session runs ahead of the player: by the time the player notices, the new timeline's samples were already judged against the old join state (`LiveJoin` `Keep`) and the media ends reported against the old controller state; with a burst delivery the next `JoinPoint` and `Start` never came. Module `src/timeline.rs` (`JumpDetector`), wired in `session.rs`.

Rule (`Config::timeline_jump: Option<TimelineJumpConfig { backward_us: 1_000_000, forward_slack_us: 5_000_000 }>`, default `Some`, `None` disables; also on for VOD): per video track (a track with a `LiveJoin`), between CONSECUTIVE KEPT samples (join decision not `Drop(_)`; `Drop`s are neither compared nor remembered), with `step = pts_i - pts_prev` converted to microseconds from the track timescale, a new timeline starts when `step < -backward_us` or `step > elapsed_us + forward_slack_us`. `elapsed_us` is the session `now_us` at which sample i was delivered minus the one of sample i-1 (the caller's monotonic clock; samples demuxed from one `on_data` share its `now_us`, so inside a burst any forward step above the slack is a jump). Both comparisons are strict. Audio uses the same rule only for the first RENDERED audio track (explicit `set_rendered_tracks` set, else the first announced) and only when no video track was announced. B-frame reordering (kept samples in decode order step backwards by a few frames) never reaches -1 s; the real TF1 capture, which reorders, is asserted jump-free.

On detection, in sample order and before the sample is judged or emitted: (1) `Event::Discontinuity`; (2) the track's `LiveJoin` is re-armed and pending ends/maxPTS/join origin are cleared as `rejoin()` does, and the sample is judged again by the fresh state (so `Event::Sample::join` reflects the new timeline: `Drop(_)` or `JoinPoint`); (3) `Hint::PcrDiscontinuity` goes to the controller exactly as the demuxer-driven path does (live TS only: HLS and RTSP have no controller and never create one); (4) the BC-5 playout gate resets as for a `Discontinuity`; (5) the per-track last pts/last now are forgotten. RTSP samples carry `join: None` and are all "kept"; for HLS (2) re-arms the per-track `LiveJoin` the session keeps (Audit round 5 F5) and (3) is a no-op.

Deduplication and state scope: every `Event::Discontinuity` (source reconnect, HLS discontinuity, variant switch, a detected jump) and `Session::rejoin()` forget the detector state, so the first sample after them compares only within the new timeline and no second `Discontinuity` follows. A demuxer discontinuity (`DemuxEvent::Discontinuity`, PCR flag or PCR jump) is host-invisible today (it rejoins and hints only); the detector state is deliberately NOT cleared by it, so a PTS jump that coincides with it still produces the one host-visible `Event::Discontinuity`, but the rejoin and the hint are not repeated for the same sample (`handled` flag until the next kept sample is recorded).

Controller semantics worth knowing: `buffering-runtime` answers a `PcrDiscontinuity` while Playing with `Pause` then `Resume`, never a second `StartPlaying`, so after a jump the player sees `Resume` (or `Start` if the controller was still Buffering) positioned on the new timeline's media. The BC-5 gate does fire `Start` again.

Tests: `broadcast-client/tests/timeline_jump.rs` (TS via PES-PTS rewriting of the public `h264_aac.ts` and the real TF1 capture, HLS VOD/live through the real HLS session, RTSP through the fake server with rewritten RTP timestamps, a two-video-track RTSP SDP) and the exact-boundary unit tests in `src/timeline.rs`.

## Audit round 4 (#1498)

- **Reconnect re-arms the controller.** A `Discontinuity` from a new TS connection re-arms the live
  join, clears the pending media ends, the maximum timestamps and the join state (`Feeder::rejoin`)
  before the `Reconnected` hint, so the reconnected stream waits for an IDR (`Drop(BeforeRandomAccessPoint)`)
  and the controller measures the new timeline (lower, equal or advanced base).
- **Timeline rebasing.** The feeder and the playout gate convert time stamps to 27 MHz in `i128`
  relative to a per-timeline origin: the first kept sample (video join point, else audio), taken as
  `min(0, pts)`. A timeline that starts at or above zero is converted unchanged (positions are the
  sample-time domain); one whose first kept sample is negative is measured from that sample and never
  clamps to a flat zero. Only overflow of `u64` clamps.
- **Stale tracks after a jump.** A detected jump tags every other track stale; its samples are dropped
  (`Session::stale_samples_dropped`) until its first sample lies within `forward_slack_us` of the
  first sample of the new timeline.
- **PCR discontinuity plus jump.** `handled` is cleared only by a sample compared against a sample
  recorded after the reset, so one coincident event hints and re-arms once.
- **HLS redirect base.** `HlsClient::set_playlist_base(url)` (additive) sets the URL relative playlist
  URIs resolve against; the adapter passes the final playlist URL before `on_playlist`. Reloads keep
  requesting `playlist_url()`; credentials stay origin-scoped.
- **Digest `uri`.** HTTP(S) requests use the origin-form request target (RFC 7616 section 3.4); RTSP
  keeps the absolute request URI.
- **RTSP redirects.** 301/302/303/307/308 with an `rtsp(s)` `Location` are followed (bounded by
  `max_redirects`, never `rtsps` to `rtsp`), credentials only to the same authority; the redirect
  needs `ClientSession::redirect_location()`. `Event::Discontinuity` is emitted only before the first
  sample of a connection that follows one that produced media.
- **`on_response` takes `now_us`** (breaking, pre-release).
- **Played-then-ended reconnect** emits `Status::Reconnecting { attempt: 0 }` for TS-over-HTTP and RTSP
  (a caller `reconnect()` does not); HLS has no such reconnect (its reload chain continues).
- **Playout gate.** The position reported before `Start` is discarded at `Start`, so a host that keeps
  reporting the position of the previous pass cannot pause the new pass at once.
- **Host contract.** poll until `None` and drain after every callback; live TS decides `Buffer`
  events inside `on_playback_position` (never called: no `Start`; called less than every display tick:
  `Start` later, not never; pinned by `tests/audit_r7_buffer.rs`); `Start.at`/`Resume.at` are the 27 MHz clock the player
  sets; demuxer-level discontinuities are visible only through buffer events; `byte_range` is
  `(offset, length)`; `WakeAt` is "at or after" and past values are dropped.

## Policy core split (tshttp-runtime, story mm-tshttp)

`tshttp-runtime` gains a demux-free core, `policy::TsHttpPolicy`, and `TsHttpClient` becomes the core plus a
`transmux::StreamingTsDemux`. The core sees only response headers, chunk LENGTHS, a "a sample was produced"
flag and timestamps; it never sees TS bytes. Transition numbers are those of `docs/tshttp-runtime/DESIGN.md`.

**Moves into the core (all decision logic):** T1 (new -> Open), T2, T3, T4 (response status handling, errors),
T5 (Awaiting -> Playing on the sample flag), T6, T7, T8, T9 (backoff schedule), T10, T11, T12, T13, T14, T15,
T16, T17, T18, T19, T20, plus finite/VOD detection (Content-Length / 206 Content-Range / identity coding) and
`next_deadline`. Core inputs: `on_response`, `on_bytes(len, now, sampled)`, `on_sample`, `on_eof`, `on_error`,
`reconnect`, `poll_timeout[_with]`. Core outputs: `Action`s and `Event`s (never `Event::Demux`).

**Stays in the client (everything that touches TS bytes):** the `StreamingTsDemux` itself; creating a fresh demux
on every `Open` (observed through `TsHttpPolicy::open_count`); gating `feed` on `accepts_bytes`; draining demux
events into `Event::Demux` (through `push_event`, so ordering against policy events is unchanged); flushing the
demux (`finish`) before the terminal decisions T6/T7/T8/T12/T13 and reporting whether the flush produced a
first sample (the core takes that as a callback because a flushed sample changes the T6/T12 outcome).

**Redirects and auth challenges are NOT in the core** (they live in the broadcast-client `Session` layer,
unchanged): the core never sees a 3xx/401/407.

**multimux adopts the core only** (`multimux/src/source/ts_http.rs`); it keeps `ts_program` for demuxing and
the supervisor for reconnect/backoff: every core `Action::Open/Reconnect` and `Event::Reconnecting` backoff is
ignored by multimux (no second retry loop). Mapping: `Event::Ended` -> clean end (`HealthState::Ended`);
core `Abort` (T6/T12 stall) or a failure verdict (T7/T19 truncated transfer, T9) -> the existing
`MultimuxError::Connect` failure; `Event::Starved`/`StallNotice` -> `tracing` only. "sampled" for multimux is
"the ingest driver is `Live`" (route established).

**multimux behaviour changes (all listed):**
1. Finite responses (Content-Length / 206) are now detected from the response headers; EOF after all expected
   bytes is `Ended` (unchanged outcome), EOF before them is a failure (`Connect`, reason "truncated transfer")
   instead of `Ended`. With reqwest/hyper a short Content-Length body already surfaces as a read error, so this
   only formalises the existing outcome; no user-visible difference is expected.
2. The post-establishment stall threshold is decided by the core, configured from `IngestTimeouts::read`
   (same value, same error text `ts/http stream read: no data within ..`). The wait inside one step is also
   capped at the core's next deadline. The scaffold's own read-stall check stays as the pre-establishment guard.
3. The core's START timeout is DISABLED in multimux (`u64::MAX`): enforcing it would newly fail a stream that
   delivers bytes but no PMT, a user-visible change (today the handshake deadline is unbounded). Stall notices
   are derived (`stall_reconnect / 2`) and only logged.
4. Redirects: multimux already follows redirects (`source::redirect_policy`: 4 hops, no https -> http, credentials
   scoped to the original origin). No change; a test pins it. The core does not implement redirects.

None of these is a user-visible change in a supported flow, so the story proceeds.

## Audit round 5 (r5client)

### F5: HLS samples carry a join decision

The HLS source used to emit every sample with `join: None`, so a stream whose first segment starts
mid-GOP (a live join that is not PAT/IDR-aligned) delivered a non-sync first video sample and the host had
to find the random-access point itself. The session now keeps one `transmux::LiveJoin` per video track of an
HLS source (`source/hls_join.rs`, `LiveJoin::for_config` over `Event::Tracks`, the same helper the
TS-over-HTTP path drives from `buffer.rs`). Every video sample is judged in delivery order before the
timeline-jump check and the playout gate see it: the first random-access point is `Some(JoinPoint(_))`,
everything before it (and the leading pictures the policy rejects) is `Some(Drop(_))`, later samples are
`Some(Keep)`. As everywhere in this crate the session still emits `Drop` samples (it never drops samples);
the host skips them, and the BC-5 gate and the jump detector already ignore them. The join is re-armed by
every `Event::Discontinuity` (HLS discontinuity, variant switch, a detected timeline jump, which re-judges
the triggering sample) and by `Session::rejoin()`. Audio and other non-AVC/HEVC/VVC tracks (including the
BC-3b audio renditions) keep `join: None`. On a conformant PAT/IDR-aligned stream the first video sample is the
join point and nothing is dropped, so the gate's behaviour is unchanged; after `Session::rejoin()` the gate
now restarts at the next random-access point instead of the next video sample (the same rule as TS).

### F4: HLS-TS tracks that cannot be muxed are reported

`hls-runtime` re-muxes TS segments into fragmented MP4 and leaves out every elementary stream
`!is_muxable_in_bmff` (opaque `Data` tracks: SCTE-35, ID3, private PES; subtitles without a sample entry).
That used to be silent: a data-only stream reached `Status::Ended` with no samples and no explanation, and a
stream with a PMT-listed data PID played without telling the host about the missing track. `hls-runtime` now
emits an additive `Output::TrackDropped { track_id, stream_type }` from that branch of `drain_ts_demux`, and the
HLS source turns each one into ONE non-fatal `Event::Error { kind: ErrorKind::TrackDropped { track_id,
stream_type }, fatal: false }`, deduplicated by (remapped) track id for the life of the source. The existing
`ErrorKind::UnsupportedCodec { fourcc }` was not reused: it names a codec by FourCC and is documented as a
player-side kind, whereas these tracks have no FourCC (they are PMT stream types) and the host needs the track id
to correlate. `ErrorKind` is `#[non_exhaustive]`; the new kind has `name()` `"track_dropped"` and `Display`
via `impl_spec_display!`. Media of the muxable tracks flows unchanged. Audio renditions report the remapped id
(`id + AUDIO_TRACK_ID_OFFSET`). Evidence: `fixtures/ts/scte35-other-pid.ts` (data-only) reports its data tracks
and ends; the TF1 capture's two `stream_type` 0x06 tracks yield two reports while video and audio play.

### F1: the initial TS-over-HTTP track set is announced once

A live TS announces its tracks as each stream's codec config resolves (the first SPS, the first ADTS header), so a
host that builds its pipeline from the first `Event::Tracks` started video-only (`ts/france2.ts` in 64 KB chunks:
`[1]`, then `[1,2]` after 7 samples, the full six after 73; the 30-stream France TNT multiplex produced ~30
announcements). The source now HOLDS the first media of every connection: from the first `Sample` or
`DisplayChanged` it queues every item in order (`Sample`, `DisplayChanged`, discontinuity markers, `Status`
events) without delivering them, and releases them behind ONE `Event::Tracks` carrying every track resolved so
far. Release happens when the first of these holds:

1. every PMT-declared elementary stream has resolved, or every declared stream that has SENT DATA has resolved
   and the settle has elapsed: a shadow PAT/PMT reader (`source/ts_http_hold.rs`:
   `mpeg_ts::resync::TsResync` + `dvb_si::demux::SiDemux`, fed only until release) learns the programmes the PAT
   lists, the elementary PIDs their PMTs declare (all programmes: the demuxer demuxes all of them) and the PIDs
   that carried at least one payload packet (`sent`); a stream counts as resolved when the demuxer reports
   `TrackAdded`/`TrackUpdated` with that `source_pid` or a `TrackAbandoned` with that PID; the PAT must also have
   had every listed PMT seen. A declared PID that has sent nothing is never waited for (the declared-but-silent
   data PIDs of the real zenith channels: stream_type 0, 11, 12 on rai-gulp, rai-news-24 and arte, each of which
   cost every channel the whole wall limit before). `SETTLE_US` (500 ms) is how long a stream that has not
   sent yet is still waited for: it is counted from the later of the hold's start and the last resolution, as
   media time (largest per-track pts span among held samples) OR session-clock time, whichever passes first,
   and armed through `next_deadline_us`/`WakeAt`. Measured gaps between consecutive resolutions: audio resolves
   0.48 s of media after the first video in `ts/h264_aac_40s.ts`, 0.32 s in `ts/france2.ts`, 0.16 s in the TF1
   capture, 0.32 s (rai-gulp) and 0.21 s (rai-news-24); the slowest sparse stream_type 6 PID resolves 0.44 s
   after the previous one (`ts/france2.ts`). A stream that has sent but not resolved (scrambled, sparse) is
   waited for until a limit below;
2. a limit, counted from the first media of the connection: `HOLD_QUIET_INPUT_BYTES` (512 KiB of input with no
   further stream resolving: the scrambled `csa/csa2-plain.ts` leaves declared streams unresolved forever and
   round-5 F2's liveness test needs media within 0.56 MB of it), `HOLD_MAX_INPUT_BYTES` (4 MiB of input),
   `HOLD_MAX_MEDIA_US` (1 s of media, the largest per-track pts span among held samples), or `HOLD_MAX_WALL_US`
   (1 s of caller clock, armed through `next_deadline_us`/`WakeAt`, so a source that goes quiet mid-hold still
   releases; the 3 s defaults of the first version cost the owner's Apple TV 2.29 s to first media against
   0.07 s with no hold, 1.09 s at 1 s);
   The limits are `Config::tracks_hold_quiet_input_bytes`, `tracks_hold_max_input_bytes`,
   `tracks_hold_max_media_us` and `tracks_hold_max_wall_us`; the values above are their defaults.
   A zero wall or media limit disables the hold (tracks are announced as each stream resolves, the
   pre-round-5 behaviour).
   `Session::open` rejects a zero quiet limit and a max-input below the quiet limit with
   `Error::InvalidConfig`;
3. anything that must not overtake the held media: end of stream, a stall notice, a reconnect, a new connection,
   a 404, a starved report.

Evidence for the numbers (release point per capture): single programme `ts/france2.ts` 0.76 s of media / 0.85 MB,
the TF1 capture 0.74 s / 0.26 MB, `ts/h264_aac.ts` within the first chunk, the 30-stream
`france-tnt-uhf32-video-head-6MB.ts` 1.76 s / 1.5 MB when allowed to run to resolution; its 0.92 MB gap between
two resolution bursts exceeds the quiet bound, so that multiplex is announced in several steps (first set, then one
`Tracks` update per later track, the pre-F1 behaviour) rather than once. The limits leave ~1.7x headroom over the worst case and
stay under the 5 s stall notice. A stream that declares a PID it never sends (modelled by dropping `PID 142` from
`france2.ts`) is released at most one settle after the last resolution (it used to wait out the limit); only a stream that
sends without resolving is released at the limit, with the streams that did resolve. Tracks that resolve after the release
announce as updates exactly as before (`Event::Tracks` with the full merged set). Samples are never lost or
reordered: the held items are appended behind the `Tracks` event in arrival order (tests compare the session's
sample sequence with a bare `StreamingTsDemux` over the same bytes). `Status::Live` keeps its place in the
stream (after the first `DisplayChanged`, before the first `Sample`), so it is now delivered at release rather
than at the first sample. A reconnect starts a fresh hold for the new connection.

Consequence for BC-2: the buffering controller learns the tracks and the media ends from the release on (the
held samples replay in order within one drain), so its first `TrackMedia` and the zero-length join report can
arrive up to the hold later than before; chunk arrival pacing (`on_chunk`, PCR) is unaffected. The oracle in
`tests/buffer.rs` models the same hold. RTSP announces all tracks from the SDP before the first sample, so it
needs no coalescing.

### multimux check: TS-HTTP bytes without media

`multimux`'s TS session is `Established` at construction (the HTTP response is already in), so the driver is
`Live` from the first byte and the handshake deadline stops mattering once any byte arrives; the policy core's
start timeout was disabled (`u64::MAX`, item 3 of the policy-core section), and its `sampled` input was "driver
`Live`", so the core entered `Playing` on the first byte. A server that sent a PAT and went silent was therefore
only aborted by the scaffold's `IngestTimeouts::read` check, and one that kept sending a PAT without a PMT was
never aborted. Loopback tests (`multimux/src/source/ts_http.rs`: `a_server_that_goes_silent_before_the_pmt...`,
`a_server_that_keeps_sending_without_ever_sending_a_pmt...`, 1 s connect budget, 30 s read) failed against the
unfixed code (the outer window fired). Fix: the core's start timeout is now `IngestTimeouts::connect`, and
`sampled` is "`driver.trunk(ProgramId(0))` exists" (a program was announced), so a route with bytes but no media
fails with `ts/http handshake timed out: no media within <connect>` and the supervisor reconnects. This revises
item 3 above: the previously avoided user-visible change (a stream that delivers bytes but no PMT now fails after
the connect budget, default `DEFAULT_CONNECT_TIMEOUT`) is accepted deliberately.

## Round 5: live liveness of the TS demux path (F2, F3, F6)

Decisions that change public behaviour (transmux `StreamingTsDemux`, hls-runtime TS client):

- **F3, head-of-line blocking.** A `Probing` PID blocks only the PIDs of its own program (the declaring PMT). Across programs, promotion is by resolution order, so a live TS-HTTP source delivers as soon as any program resolves; track ids are therefore not in PMT order across programs. Inside a program PMT order is kept, with the existing step-over bounds plus two new ones: a head with 2 MiB of its own payload and no access unit (`STUCK_HEAD_OWN_BYTES`), and a non-video head 4 MiB of input after its declaration (`STUCK_HEAD_INPUT_PACKETS`). Video that yields access units stays bounded by the 4 MiB backlog cap. The stepped-over PID keeps probing and is announced later with a later id. Whole-buffer `TsDemux` keeps strict PMT order (`with_strict_pmt_order`), which is what `track_id_baseline` pins.
- **F2, live HLS.** Evidence: `m6-single.ts` and `tnt-5w-12732v-isi6-10s.ts` declare video PIDs that never send a packet, so no byte bound can fire on a segment-sized input and the 9000-packet late-video regression forbids a small one. A whole segment is the one place absence is a fact, so after each `Segment` (not each LL-HLS `Part`) the client calls `release_silent_heads`. A PES that spans a segment boundary is untouched: nothing is flushed, so the sample sequence of PAT-aligned `tf1-live.ts` segments equals the whole-file demux byte for byte (`pat_aligned_segments_deliver_the_whole_file_demux_byte_for_byte`).
- **F6, discontinuity.** The fresh demuxer after `EXT-X-DISCONTINUITY` re-probed from the new timeline alone; a segment that opens mid-GOP carries no SPS for some PIDs, so those tracks vanished (france-tnt video head 17 to 15 tracks). The client now calls `restart_timeline`: tracks, ids and parameter sets are kept, timing is reset. Measured cost against a no-discontinuity run on PAT-aligned cuts: at most one audio PES per PID (6 to 7 frames). Not recovered, by design: a PID that was still unresolved at the seam loses its old-timeline backlog (those access units precede the PID's first parameter set, so they are undecodable anyway: in `test-092.ts` PID 0x26C the first sync sample is the 147th of 149), and a PID whose config only exists across the seam stays unannounced (`csa2-plain.ts` PIDs 0x276 to 0x278). A changed PMT after the seam is diffed as usual (tracks added, removed or updated).

### Interaction with the round-5 liveness fixes (#1502)

`tests/audit_r5_liveness.rs` and `tests/audit_r5_discontinuity.rs` assert "no `Event::Error`". F4 deliberately
reports each non-muxable HLS-TS track (SCTE-35, ID3, private data present in those captures) as a non-fatal
`ErrorKind::TrackDropped`, so both files now ignore that one kind when collecting errors (nothing else in them
changed). The F1 hold interacted with the TS-HTTP liveness test on `csa/csa2-plain.ts` (declared streams that never
resolve): the hold had no input-driven bound below 4 MiB / 3 s, so nothing was released inside the test's 0.56 MB;
`HOLD_QUIET_INPUT_BYTES` fixes that.

### Hold transparency and the timeline-jump epoch

`tests/hold_transparency.rs` replays every `.ts` fixture (public and private, 20+ producing samples) paced at its
PCR bitrate with the hold at 0, 0.3 s, 1 s and 3 s and requires the delivered `Event::Sample` sequence (track, pts,
dts, sync flag, join decision, data hash) to be identical to the disabled hold. It found one difference: after a
detected timeline jump the stale set (tracks whose old-timeline samples are dropped until they reach the new
timeline) was every track the jump detector KNEW, and the hold announces all tracks before the first sample, so
tracks that had delivered nothing yet had their first samples dropped (`ts/m6-single.ts`: 17 of 22 samples,
TF1 capture: 51 of 75) where the disabled hold delivered them. The stale set is now the tracks that delivered a
sample on the old timeline (`JumpDetector::delivered`, reset at each epoch).

With a host simulation (Buffer `Start`/`Pause`/`Resume` applied, samples behind the playout clock counted late) the
real captures show no late sample at any hold setting. Before the early release, rai-gulp's first `Tracks` came at
0.33 s / 1.33 s / 2.61 s for hold 0 / 1 s / 3 s with `Start` at 1.86 s / 1.96 s / 3.28 s and a burst of 229 / 242 / 401
samples before `Start`, the sample sequences byte-identical. With the early release the first `Tracks` comes at
0.76 s for a 1 s AND a 3 s hold, `Start` at 1.86 s and a 229-sample burst in all three (rai-news-24: 1.13 s,
Start 2.68 s, 224 samples). So the 1439 decode failures the Apple TV reported for the 3 s hold are not caused by
the released samples (identical bytes, order and flags to the no-hold run, which plays clean); the host must
tolerate a burst of up to the hold length, and the early release removes that burst for these channels.

## Start gate: a declared stream with no track yet (TS-HTTP)

zenith-tv (Apple TV, defaults, `[video]` + `[audio]` declared) saw `Start` after the 1 s wall backstop released
the hold, then a `Pause` 100 ms later with the audio 8 ms ahead of the playback position and the video 1.1 s ahead.

Root cause. The start rule and the pause rule are the same: both use the minimum end over the controller's
rendered tracks. The inconsistency is the SET the minimum is taken over. A live mux sends audio up to a second
behind the video and an audio track is announced only once its first frame resolves its codec config, so at the
moment the video has its start lead the audio has no `TrackSpec`: it is not in `track_specs`, an explicit
declaration of it is pending (`rendered.rs`), the controller has never heard of it, and `Start` fires on video
alone. The audio is announced afterwards, joins the set, and its end sits roughly `skew` behind the video end, so
`start lead - skew` is its lead over the position: at or below `starve_threshold` (20 ms) the controller pauses.
It is not hold-related: with the hold disabled, or released early, the same flap happens (rai-gulp with the audio
muxed 500 ms later and a 1.1 s start lead: `Start` at 0.70 s, `Pause` at 0.85 s, audio lead 6 ms, in all three
hold settings and with or without an explicit declaration). With the default 1.8 s start lead and the natural
skew of the committed captures (0.3 s to 0.5 s) the audio is already announced when the video is ready, which is why
the soak baseline never saw it.

Rule (single, `buffer.rs`): `Start` fires only when every track the stream is known to carry has its start lead.
The source keeps the PMT's audio/video streams (`ts_http_hold::StreamKind`: video and audio `stream_type`s, and
`stream_type` 0x06 with an AC-3, E-AC-3, AAC or DTS descriptor) and which of them have resolved to a track or been
abandoned (`TsHttpSource::unresolved_kinds`). When a kind is declared but unresolved and no track of that kind is
announced (and, for an explicit declaration, a declared id is still pending), the session tells the feeder
(`Feeder::set_expecting`) and the feeder registers a placeholder controller track (`PENDING_TRACK_KEY`) with the
first media report. The controller then does what it already does for any registered track that has not
delivered: no effective end, so no `Start`, until the real track is registered and delivers, which brings the
unchanged min-lead rule. The wait is bounded, from the first media report, by `Config::start_pending_track_wait_us` (default 1.5 s) and, while a PMT-declared audio stream is still unresolved, by the larger of that and `Config::start_pending_track_hard_cap_us` (default 4 s). `0` on the wait does not wait, `0` on the cap adds no extra cap, and `u64::MAX` waits until the first `Start`; a host with a short start lead and a tight first-frame budget sets 0.5 s to 1 s on the wait and `0` on the cap. A declared audio stream that never delivers therefore delays `Start` by up to the cap. The wait ends at the first
`Start`; after it `Start` goes ahead with the tracks that exist. Default and explicit declarations behave the same
because the expectation comes from the PMT, filtered by the declaration only to leave out a stream the host never
asked to render. `buffering-runtime` is unchanged, but its `track_silence_timeout_us` would silently cap any wait longer than it (a 10 s wait started at about 3.8 s with the 3 s default), so the feeder raises the controller's silence timeout to at least the effective wait plus 500 ms when it builds the controller. Signalling: `Status::AwaitingTracks` is emitted once when a hold begins (audio still unresolved at the first media report; never when every track resolved before it, never again within the same session), and `ErrorKind::PendingTrackUnresolved { kind: StreamKind::Audio }` is emitted once, non-fatal, when the wait expires with audio still unresolved before the first `Start`; it is not emitted when audio resolves in time or when the wait is `0` (no hold). Streams whose tracks all resolve before the first media report
(every committed capture at its natural skew) never show the placeholder to the controller: their controller
input log, and so every existing oracle test, is unchanged.

Evidence (`tests/start_gate.rs`, host simulation at real time with a 50 ms tick, captures rai-gulp, rai-news-24,
`ts/h264_aac_40s.ts`, `ts/france2.ts`, hold early / 200 ms backstop / disabled, declared `[1, 2]` and default, the
audio PES muxed 0 / 500 / 1000 ms later): before, every skewed run with a 1.1 s start lead flapped (`Start`
0.70 s to 1.25 s, `Pause` 0.15 s to 0.6 s later); after, none flaps and `Start` carries both tracks at at least 90%
of the start lead. At the default 1.8 s start lead and at skew 0 every `Start` time is identical before and after.
With a 1.1 s start lead the skewed `Start` moves by the skew actually present plus the audio's own lead: rai-gulp
skew 500 0.80 s to 1.80 s, skew 1000 0.80 s to 2.25 s; rai-news-24 skew 1000 1.25 s to 2.75 s;
`h264_aac_40s` skew 1000 1.00 s to 2.10 s. `ts/france2.ts` (2.7 s long) never reaches the audio's lead at skew 1000
and 1.1 s. The captures `tf-1-8MB.ts` and `arte-8MB.ts` are not used: 4% of their packets are missing (continuity
errors on the video PID) and no track resolves for 13 s of real time.

## Audit round 6 (session, TS-HTTP, buffer, RTSP)

### F2: a new TS-HTTP connection rebuilds the track set

`StreamingTsDemux` is replaced on every connection and restarts its ids at 1, but `TsHttpSource` kept its
`tracks` map across connections, so the first `Tracks` after `reconnect()` was the union of both connections
(tf1-live then h264_aac announced `[1..6]`; h264_aac_40s then a video-only stream kept audio `2` as a phantom in
`default_rendered_tracks()`). `Connected` now clears the map and marks it dirty, so the first sample of the new
connection announces its own tracks only; the session's `retain_tracks`, jump detector and playout drop what
vanished. RTSP does not share the flaw (the track set is rebuilt from each DESCRIBE); a test pins it.

### F4: chunk invariance (no defect found in the session)

The reported 479/477/478/467 sample counts came from replaying `tf-1-8MB.ts` at the PCR-estimated bitrate (1.87
Mbps, wrong for a capture with a PCR jump): the first sample of that capture is 3.9 MB in, so the 15 s start
timeout expired at a read-quantised byte offset, reconnected, and the demuxer restarted at a different place per
read size. With clocks that do not trip a timer, `tests/r6_chunk_invariance.rs` replays tf-1, arte, rai-gulp and
five public fixtures at reads of 188, 1316, 4096, 16 KiB, 64 KiB, 256 KiB and three random sequences, under a fixed
clock, a 40 Mbps clock and a 6 Mbps clock, and asserts the `Sample` sequence and the `Discontinuity` positions
identical (and the final track set, and that every sample follows an announcement naming its track). The raw
`StreamingTsDemux` is chunk-invariant on the same inputs. Not asserted, because arrival time decides them:
`Buffer` events, `Status`/`Starved`, and the grouping of `Tracks` (the hold releases at the first read
boundary at which its condition holds). A mutation that drops tail packets of large reads fails the test.

### F5/F6: the explicit rendered declaration

`src/rendered.rs` resolves the host's declaration against the announced set (live TS-over-HTTP only; HLS and
RTSP pass it through, because the HLS gate legitimately waits for a separate audio rendition that is announced
late). Unknown ids are ignored, remembered, applied when announced and reported once per declaration as
`ErrorKind::UnknownRenderedTrack`; a wholly pending non-empty declaration renders the default set. For a PMT
drop and restore transmux returns the stream under a NEW `TrackId`. Reusing the id inside `StreamingTsDemux` was
rejected: ids come from one monotonic counter that every consumer (`media-plane` trunks, the HLS/TS segmenters)
keys state by, and `remove_track` documents the churn as intended (#774); reusing an id would silently splice a
returning stream onto a removed track's state. The least invasive fix keeps the id contract and makes the
declaration kind-aware: a declared video or audio id that vanished is replaced by the lowest announced track of
its kind that the host did not declare. `pmt_flip` on `h264_aac_40s.ts` (audio dropped at 40 %, restored at 70 %)
announces `[1,2]`, `[1]`, `[1,3]` and the controller registers 1, 2, unregisters 2, registers 3.

### F7: every video codec

`transmux::CodecConfig::is_video()` is now public. The client keys "video" off it (default rendered set, jump
detector roles, playout kinds) instead of `LiveJoin::for_config`, which is `None` for MPEG-2, AV1, VP9 and VP8.
`join.rs` wraps `LiveJoin` for AVC/HEVC/VVC and adds a sync-sample join for the rest: the first sync sample is the
join point, earlier samples are `Drop(BeforeRandomAccessPoint)`, samples before the join point in presentation order
are `Drop(LeadingPicture)`. TS and HLS use the same wrapper. `legacy/mpeg2_mp2.ts` plays with a join point and
the video is the default rendered video; the AV1 and VP9 fragmented MP4 fixtures join at their first sync sample.

### F8: RTSP codecs

A section that is not played is reported once per source as a non-fatal `TrackDropped` (id after the played tracks,
`stream_type: None`). An SDP with no playable section whose skipped section names an unsupported `rtpmap` codec
ends with ONE fatal `UnsupportedCodec { fourcc: <encoding name> }`; the source closes its connection and
`reconnect()` is ignored afterwards. Other nothing-playable SDPs (no codec named, a played codec with an unusable
`fmtp`, unparsable) stay ordinary failed attempts. The old audit test that required a reconnect loop for
PCMU+application media now asserts the fatal error. H.265 over RTP is NOT cheap: `transmux::RtpMediaKind` has
only H.264 and AAC, so an RFC 7798 depayloader is a follow-up in transmux.

### F9b: Config validation

`Config::validate` (run by `Session::open`) rejects `max_queued_events` 0 (every sample would be dropped),
zero HLS size limits and timeouts, a `buffer` the controller refuses (the feeder used to swallow that error and
never create a controller, so no `Buffer` event ever came), and zero timeline-jump thresholds (every reordered
frame would be a jump and the other tracks' samples dropped as stale). Documented, not rejected: `max_redirects` 0
follows none, `max_key_failures` 0 behaves as 1, `u64::MAX` jump thresholds never fire, a playout lead the stream
cannot reach keeps the gate closed, `ts_http` values are clamped (`tshttp_runtime::Config::clamped`). The fuzz
target now randomizes the Config knobs (header of 8 bytes), declares unknown ids and `u32::MAX`, reads from four
real TS fixtures plus a PMT-flipped copy, and `fuzz/seed-corpus.py` generates six scripted scenario seeds.

## Audit round 6 (r6hls): HLS/fMP4 path

### F1: skipped fMP4 tracks are reported, an all-skipped stream is fatal
`Fmp4Demux` records a track it cannot model in `Media::skipped` (now with the `tkhd.track_ID`, `transmux::SkippedTrack::track_id`). hls-runtime forwards each as `Output::TrackSkipped` (once per track and init), and the HLS source turns it into ONE non-fatal `Event::Error` per track id per session: `ErrorKind::UnsupportedEncryption { method: "encv" | "enca" }` for an encrypted sample entry (CENC/CBCS sample encryption is never decrypted; the playlist tags are not consulted, a plain playlist can carry an encrypted init), `ErrorKind::TrackDropped { track_id, stream_type: None, fourcc: Some(..) }` for any other. `TrackDropped` gained only the additive `fourcc: Option<String>`. When the init yields no track at all (empty `Tracks` after at least one skip) the session ends with a FATAL error of the first skipped track's kind instead of `Ended` with zero samples.

### F3: one init per `EXT-X-MAP` run
RFC 8216 §4.4.4.5 (`broadcast-hls/docs/playlist-tags.md`): a map applies to every Media Segment after it until the next map. `HlsClient` keeps an `InitSlot` per map-run (identity: resolved URL + `BYTERANGE`), binds each Media Sequence Number to the slot of the map that precedes it (`msn_init`), and demuxes every Part/Segment with its own slot's bytes; a resource that arrives before its init waits in `pending_demux`. Because `ResourceId::Init` carries no identity, inits are fetched one at a time in playlist order. The consumer-visible contract: `Output::Init` for the first init on arrival, every other init lazily before the first resource that uses it (after that resource's `Discontinuity`), then `Tracks` when the set changed. The key of a run's init is the `EXT-X-KEY` preceding that run's `EXT-X-MAP` line; an unsupported one skips only that run's segments. Multi-MAP windows (backlog item 15 remainder) are covered by the same state.

### F9a: typed errors instead of silent zero-sample endings
- A progressive MP4 served as init and segment (`moov`, no `moof`) -> `Error::NotFragmented` -> FATAL `ErrorKind::UnsupportedContainer { reason: "non_fragmented_mp4" }`.
- A fragment the demuxer skipped (`Media::degraded` `MalformedUnit`): notably an absolute `tfhd` base-data-offset whose target lies outside the segment's own bytes (`mp4/prft.mp4`, `mp4/frag/mpeg2video.frag.mp4`, `mp4/cmaf/av_frag.mp4`, `mp4/llcmaf/ll_chunked.mp4` split into segments; CMAF uses default-base-is-moof). The good fragments' samples are emitted, the loss is reported ONCE per session as non-fatal `UnsupportedContainer { reason: "damaged_fragment" }`. Documented limitation, not a fix: such a layout only resolves when the segment sits at its original file offset.
- A playlist or segment body over its cap closes the request (as before) and now reports ONE non-fatal `ErrorKind::BodyTooLarge { field: "max_manifest_bytes" | "max_segment_bytes" }` per field and session; the retry meets the same cap, so the host should raise it. (A key body over `max_key_bytes` already reported `KeyFetchFailed`.)
- A `SUBTITLES` rendition in a master is never fetched or played; it is listed in `Event::Variants`' renditions and playing it is the host's job (README host contract). No per-track event: there is no track.

## Audit round 7 (r7fix)

### F1: a failed `EXT-X-MAP` init fetch is retried and bounded
A VOD playlist whose init fetch failed once stalled forever: `fail_purpose` retried a failed init by refetching the playlist, but a VOD reparse does not requeue a `Failed` init slot (`hls-runtime`'s `retry_init()` exists for exactly that and the source never called it), so the session sat in `Reconnecting { attempt: 1 }` with no error. The source now remembers the failure (`init_retry`), and when the shared backoff gate lifts it calls `HlsClient::retry_init()`; the playlist refetch is skipped once `EXT-X-ENDLIST` is seen (it cannot help) and kept for live (it recovers there too). Each failed init fetch (any status including 403, transport error, timeout, oversized body) is counted by the same counter as key failures: one non-fatal `ErrorKind::InitFetchFailed { status }` while retrying, ONE fatal of the same kind after `Config::max_key_failures` consecutive failures with no decoded samples in between (the init analogue of `KeyFetchFailed`; there is no separate cap and segment fetches are bounded by their own `max_segment_failures`, Round 8). `tests/audit_r7_init.rs` covers transient/permanent 404/503, a transport error, live recovery, a failing second init of a multi-MAP VOD and the cap following `max_key_failures`. An init that arrives with a success status but holds no complete `moov` box (`Error::InitUndecodable` from hls-runtime, which also marks the slot `Failed`) takes the same path with `ErrorKind::InitDecodeFailed`: retried, counted with the same cap, one fatal. It used to surface as endless segment-demux failures. The r7 scratch test `r7_never_sent_audio_no_ticks` (F3) did tick during its idle phase; the pinned contract is in `tests/audit_r7_buffer.rs`.

### F2: `EXT-X-MAP` identity ignores the query
`HlsClient::init_for_map` matched on the full resolved URL, so a re-signed map URI created a new init slot per reload (a refetch and a repeated `Tracks`). It now matches on the URL without its query plus the byte range (the rule segment continuity already uses, `without_query`) and keeps the newest URL for later fetches and retries. A different path or byte range is a new init that is fetched and announced.

### F3: Start needs `on_playback_position`, but not every tick
The README said live TS gets `Event::Buffer` only if the host ticks every display tick. Measured: with no tick at all there is no `Start`; with ticks only after the data (a captured stream missing a declared audio) `Start` fires at 5.3 s against 1.6 s with a tick per chunk. The contract is now stated that way and pinned by `tests/audit_r7_buffer.rs`.

### F4: a PMT that changes a PID's kind replaces the expected kind
`DeclaredStreams` only inserted into its kind map and the source refreshed `expected` only when the map length changed, so a PMT that turned an audio PID into data, video or nothing left the start gate waiting for audio that was no longer declared. The map now follows the latest PMT of each program (a PID that left the PMT or lost its audio/video kind is removed) and `expected` is refreshed on any change. `tests/audit_r7_ts.rs` drives audio to data, video and removal plus the unchanged-kind (AC-3 to AAC) and data-to-audio cases, with the flip landing in a later chunk than the first PMT.

### F5: RTSP dropped-media report follows each DESCRIBE
`reported_drops` was keyed by media position and never reset, so a re-DESCRIBE with a different SDP did not report a media dropped at an already-reported position. It is now keyed by (position, codec) and replaced by the set of the current DESCRIBE: the same SDP reports nothing again, a different media at the same position is reported, and a media that returns and is dropped again is reported again. `TrackDropped.track_id` for RTSP is the 1-based position of the `m=` line (stable whatever else the SDP holds; it is not the id of an announced track).

### F6: scenarios the audit could not drive
`tests/audit_r7_coverage.rs`: byte-range `EXT-X-MAP` (whole-file `200` sliced, `206` with matching and wrong `Content-Range`, live repeated in every window, vanishing from later windows, VOD discontinuity without a new map); CENC/CBCS fMP4 without `EXT-X-KEY` (per-track non-fatal, clear audio plays) and with a `SAMPLE-AES-CTR`/`SAMPLE-AES` key tag (fatal at once); a TS-HLS `EXT-X-DISCONTINUITY` arriving while the buffer holds (`Pause`, then `Resume` at the held position, one `Discontinuity`, `Start` on the new timeline, no sample lost); a 40-stream PMT with a `stream_type` 0x06 audio declared by descriptor that never sends (`Start` is delayed by at most `start_pending_track_wait_us`). Nothing broke; the large PMT needed `tests/support/ts_pmt.rs` to span packets.


## Audit round 8 (r8fix)

### F1: a 200 that is not media is a failed fetch
A segment answered `200` with random bytes, an HTML error page or an empty body was consumed silently: `route_media_resource` buffered a non-`0x47` body forever as "waiting for its init" (a TS playlist has none), so a VOD ended `Ended` with samples missing and no error, and a live segment was lost. `HlsClient::on_resource` now returns `Error::NotMedia { id }` unless the body is recognisable for its container: MPEG-TS needs the `0x47` sync byte on at least half of its 188-byte lanes (ISO/IEC 13818-1 §2.4.3.2; a segment of only null packets passes), ISO BMFF a complete `moof` followed by a complete `mdat` (ISO/IEC 14496-12 §8.8.4) or a progressive movie (kept for the fatal `non_fragmented_mp4`). The check runs on the plaintext (after AES-128 decryption) and before the pending-init buffering. Structurally valid media that yields zero samples is accepted. The source maps the error like any failed fetch: it counts an attempt and retries on the backoff schedule.

### F2: per-segment attempts and the give-up rule
Every segment failure used to look like attempt 1 because the playlist refetch that re-requests a segment reset the shared counter (604 requests in 602 s at 2 s). The source keeps `seg_failures: ResourceId -> {count, last status}`; the backoff is `backoff_ms[count - 1]` (then the cap), the counter is cleared only when that resource is delivered to the core (`deliver_now` Ok), or when the resource leaves the window, never by a playlist delivery. A VOD playlist (`endlist_seen`) re-requests through the new `HlsClient::retry_resource(id)` (URL and byte range kept in `resource_requests`) without refetching the playlist. At `Config::max_segment_failures` (8; `0` = first failure final; `u32::MAX` = retry forever) consecutive failures: VOD is `go_fatal(SegmentFetchFailed { status })`; live calls `HlsClient::skip_resource(id)` (marks it delivered so no reload requests it again, restarts the TS timeline flushing the old one's held access units, queues `Output::Discontinuity`) and the source emits ONE non-fatal `SegmentFetchFailed` immediately before the `Event::Discontinuity` it maps from that output (samples flushed from the old timeline precede both), then releases the segments held behind it. This ends the head-of-line block of a listed-but-permanently-404 segment. Playlist reload failures keep retrying forever. Stream restarts reuse ids: a restart (signalled by `Output::Discontinuity`, including a discontinuity-sequence change) clears every per-resource failure count, so a still-failing media sequence number that an origin reuses starts counting again from zero; a delivery also clears it.

### F3: Retry-After
`429` and `503` carry `Retry-After`; the delta-seconds form is honoured for playlist, segment, init and key fetches: the wait is `max(backoff step, Retry-After)` with the header clamped to 60 s (`u64::MAX` cannot stall). The HTTP-date form was ignored here: the session has only a monotonic microsecond clock, no wall clock to subtract it from (superseded by the W-b section at the end of this file). Any other status ignores the header.

### F4: init timeout
An init fetch used the 60 s segment timeout, so an init with no response took about nine minutes to go fatal. Init fetches use `request_timeout_us` (20 s, the playlist class; key fetches already had `hls_key_timeout_us`), about three minutes with the default 8 failures.

Tests: `broadcast-client/tests/audit_r8_segments.rs` (every fault kind VOD and live, the schedule across playlist reloads, reset on delivery, the cap table, Retry-After table incl. `u64::MAX`, init stall) and `hls-runtime/tests/segment_failure.rs` (core contract). Every assertion was mutation-checked.

## Config renames (dash-runtime W-a0, Q1 + Q3)

| Old | New |
|---|---|
| `hls_backoff_ms` | `backoff_ms` |
| `hls_backoff_cap_ms` | `backoff_cap_ms` |
| `hls_max_segment_failures` | `max_segment_failures` |
| `hls_max_inflight_segments` | `max_inflight_segments` |
| `hls_request_timeout_us` | `request_timeout_us` |
| `hls_segment_timeout_us` | `segment_timeout_us` |
| `max_playlist_bytes` | `max_manifest_bytes` |
| `DEFAULT_HLS_BACKOFF_MS`, `DEFAULT_HLS_BACKOFF_CAP_MS`, `DEFAULT_HLS_MAX_SEGMENT_FAILURES`, `DEFAULT_HLS_MAX_INFLIGHT_SEGMENTS`, `DEFAULT_HLS_REQUEST_TIMEOUT_US`, `DEFAULT_HLS_SEGMENT_TIMEOUT_US` | same without `HLS_` |
| `DEFAULT_MAX_PLAYLIST_BYTES` | `DEFAULT_MAX_MANIFEST_BYTES` |

`Error::InvalidConfig { field }` strings follow the new names. `hls_key_timeout_us` and `max_key_*` stay: they are HLS-only. `Config` is `#[non_exhaustive]`: hosts build it with `Config::default()` plus field assignment.

## DASH clock and configuration surface (dash-runtime W-b)

Design source: `docs/dash-runtime/DESIGN.md` D4, 3.2-3.5 (clock), 6.3 (`ErrorKind`), 6.4 (`DashConfig`), 9 (work breakdown: W-b lands the whole surface so 0.1.0 does not ship a clock-less API that DASH then bolts onto). Only the API surface and its plumbing; no DASH source, and `SourceKind::Dash` is deliberately NOT added (design section 9: no stub variant that returns `UnsupportedScheme`; it lands with W-e).

Public API delta against the 3.5 table, minus `SourceKind::Dash`:

| Type | Change |
|---|---|
| `Session` | `set_wall_clock(now_us, unix_us)` |
| `Config` | field `dash: DashConfig`; `Default` and `Config::validate` extended |
| `Event` | `ClockSynced { source: ClockSource, uncertainty_us: u64 }` |
| `ErrorKind` | `MpdParse`, `MpdInvalid`, `UnsupportedAddressing`, `UnsupportedFeature`, `LiveEdgeLost`, `ClockUnavailable` |
| `Error::NotHls` | docs and message say "HLS or DASH" |
| crate root | `ClockSource`, `DashConfig`, `DashClockPolicy` re-exported from `dash-runtime` |
| `Action::HttpRequest` | unchanged; `method` may become `"HEAD"` with the DASH source (README host contract) |

Decisions made while building it:

- `ClockSource` already exists in `dash-runtime::timing::clock` (trust-ordered `MpdDate < Host < UtcTiming`, labelled): re-exported, not duplicated. `DashConfig` and `DashClockPolicy` live in `dash-runtime::client::config` (the design's layout: `DashClient::new(mpd_url, DashConfig)` takes it), so the future client engine and this crate share one definition. `broadcast-client` therefore depends on `dash-runtime` by path and `0.1` caret like its siblings; both are unpublished, so there is no epoch to keep pure yet.
- The anchor is stored as `WallClock { mono_us, unix_us }` (both `u64`) and mapped in `i128`: `unix_at(now) = unix_us + now - mono_us`. No input is rejected and nothing wraps; a response before the anchor maps by the same offset. The latest call wins. A non-DASH source consults it only in `retry_after_us`.
- `Retry-After` HTTP-date: `retry_after_us(status, headers, now_us, wall)`. The date minus "now" where "now" is the anchor mapped at the response instant, else the response's own `Date` header (RFC 9110 10.2.3 lets a client count from the server's clock; a skewed server clock cancels out), else the header is ignored. The result is clamped to `0..=60 s`, and the existing `max(backoff step, Retry-After)` rule applies unchanged. A value of only digits is delta-seconds and saturates (a value past `u64` clamps to 60 s instead of being ignored, and `+5` is no longer read as `5`); anything else must parse as an HTTP-date. `dash_runtime::timing::datetime::parse_http_date` accepts IMF-fixdate, RFC 850 (two-digit years by jiff's POSIX window, so no clock is needed) and asctime. A date beyond jiff's representable range (after 9999-12-30) is treated as garbage.
- `DashConfig::validate` rejects a zero `utc_timeout_us`, `utc_wait_us`, `utc_max_rtt_us`, `utc_resync_us`, `early_retry_us`, `min_mpd_refresh_us` and a `utc_wait_us` shorter than `utc_timeout_us` (the ladder would give up before one request can time out). Zero stays valid where it means "none" (`host_clock_uncertainty_us`, `availability_margin_us`, `early_grace_us`, `live_delay_us`); `u64::MAX` is valid everywhere (consumers saturate). The 6.4 table's 2 s `utc_timeout_us` is used, not the 3 s of the 3.3 prose. `max_mpd_bytes` of 4.2 is not a `DashConfig` field: the shared `max_manifest_bytes` bounds the MPD, as 6.4 says.
- `Config::validate` runs the DASH check for every source kind. `Error::InvalidConfig { field }` carries `"dash.<name>"`; the field strings are a static map in `events.rs`, pinned by a test per field.
- `Event::ClockSynced` ordering contract (documented on the variant, enforced when the DASH source lands): emitted once when the first anchor appears and again only when its source changes, never for a source that needs no clock, and before the first sample that depends on it. Here only the type exists and a test pins that no current source emits it.
- The session still reads no clock: `tests/no_handroll_guard.rs` now also bans `Timestamp::now`, `Zoned::now`, `UNIX_EPOCH`, `Utc::now`, `Local::now` and `OffsetDateTime::now` in `src/` outside tests, alongside `SystemTime::now` and `Instant::now`.

Tests: `tests/dash_clock_surface.rs` (anchor edge cases `u64::MAX`, backwards, repeated, after close; the Retry-After matrix through a whole HLS session; `DashConfig` validation per field; labels), `source::fetch::retry` unit tests (past dates, the 60 s clamp, garbage, anchor vs `Date`), `dash-runtime/tests/client_config.rs` and the HTTP-date forms in `dash-runtime/tests/datetime.rs`. The mutation table is `broadcast-client/docs/mutation-table.md`, re-runnable with `broadcast-client/docs/mutants.py`.

## Segment inactivity timeout and Range resume (#1613)

A host networking stack can stop delivering bytes for one in-flight request while parallel ones finish (tvOS `URLSession`, two or more concurrent downloads to one host, stalled at 720 KB of a 1.45 MB segment). Whole-request deadlines (`request_timeout_us` 20 s, `segment_timeout_us` 60 s) leave the pipeline blocked until they fire. The client now also watches each media request for silence.

- `Config::segment_inactivity_timeout_us`, default 10 s, validated non-zero (`InvalidConfig { field: "segment_inactivity_timeout_us" }`). 10 s is one 6 s target duration (the longest Apple recommends) plus the first-byte wait of a just-in-time packaging origin, a sixth of the segment deadline and half the request deadline, so a stall is recovered long before either.
- Scope: every media-class request of the segment-pull sources, HLS segment, part, init and key, DASH init, index and segment. Playlists, MPDs and `UTCTiming` are exempt: a blocking playlist reload (RFC 8216bis) legitimately holds its response with no bytes, and the manifest deadline bounds those. The whole-request deadlines still apply to every request.
- Inactivity is measured from the caller's `now_us`: the clock restarts when the request is sent, when its response head arrives and with each non-empty body chunk. `fetch::inactivity::Inactivity` keeps a lazily deleted deadline heap (`fetch::deadlines::Deadlines`) holding the deadline of the last arming; an entry whose request has been active since is re-armed at its true deadline and a `WakeAt` for it is emitted, so a host that sleeps on `WakeAt` wakes exactly when a silent request is due. No clock is read.
- On expiry the request gets `Action::Close`, one `Status::RequestStalled { resume }` (droppable and coalescing like the other transient statuses) and the existing failure path of its source: `Failure::Timeout` into the DASH core, `fail` in the HLS adapter. That path owns the retry count, the backoff schedule, `Retry-After` and the `Status::Reconnecting` that follows, so a segment that stalls every time ends exactly as one that times out every time (`max_segment_failures`). `max_inflight_segments` is untouched: the retried resource keeps its slot (HLS `Window`) or is reissued by the core.
- Resume rules (RFC 9110): a response is resumable when it is a `200` with `Accept-Ranges: bytes` (14.3) or a `206` (14.4), has no `Content-Encoding` (offsets would not be body offsets), and carries a strong validator: a strong `ETag` (8.8.3), else a `Last-Modified` at least one second older than the response `Date` (8.8.2.2). The received prefix and that validator are parked under the request's purpose (at most 8 parked, cleared on reset, variant switch and reconnect). When the same resource and byte range are requested again the request carries `Range: bytes=<first+received>-<last>` (`-` open-ended for a `200` of unknown length) and `If-Range` (13.1.5), built with the typed `headers::Range`/`IfRange` and sent in `Action::HttpRequest::headers` with `byte_range: None`.
- Answers to the resume: a `206` whose `Content-Range` starts exactly at `first+received`, ends where the original range ended and (when it names a validator) names the same one extends the prefix; a `200` (resource changed, or ranges ignored) drops the prefix and takes the whole body under the original request's rules (a ranged request answered `200` is sliced as before); a `416`, any other `206` or any other status to the resume drops the prefix and re-requests the whole resource at once without a second backoff (a `416` is not counted; a refused status is counted as any failure). An original `byte_range` (DASH `indexRange`/`mediaRange`, HLS `BYTERANGE`) is resumed inside itself: `Range` is absolute in the file, `start..=end` of the original `206`.
- A stall before the head, before any byte, or without a strong validator is `ResumeMode::Full`: the retry is the original request.

Tests: `tests/dash_inactivity.rs` (static fixture, `DashSim`), `tests/segment_inactivity.rs` (HLS fixture segments through a caller-clocked `Session`, one cell per origin behaviour x stall point), the `StallResume` fault in `tests/dash_matrix.rs`, unit tests in `fetch/resume.rs` and `fetch/inactivity.rs`.

## BC-7 UDP/RTP, SRT caller and client identity (#1555, #1460 SRT part, #1461)

Three additions, each its own PR. All stay sans-IO: the host owns every socket and every TLS stack, the session owns every protocol and timing decision, and time is the caller's `now_us`.

### Success criteria

A part is done only when every line of its list holds on a fresh run of the full gate suite.

**Common**
- The three parts add no tokio, no threads, no clock read and no randomness source to `broadcast-client`; the existing `no_clock_guard` and `no_handroll_guard` stay green. `cargo check` still passes for the Apple targets.
- Every new public enum is `#[non_exhaustive]` with `name()` and a `Display` (#204); every new failure is a typed `Error` or `ErrorKind`, never a string; no new public API takes or returns raw wire bytes except the datagram and write payloads that `Action::Send` and `Session::on_data` already carry.
- Secrets (SRT passphrase, entropy seed, client private key) never appear in `Debug`, in an `Event`, or in a log line; the workspace `secret_debug` guard covers the new types.
- Every new behaviour line has a named test on real data: TS captures read through `test_fixtures`, never hand-built bytes alone. Each test that guards a decision is mutation-checked (the mutation is shown to fail it).
- Events are the existing vocabulary. UDP, RTP and SRT sources deliver `Tracks`, `Sample`, `Status` (`Establishing`, `Live`, `Stalled`, `Reconnecting`, `Ended`), `Starved`, `Discontinuity` and the buffer/playout events exactly as TS-over-HTTP does, because they reuse its demux, hold and retry policy; only the transport differs.

**UDP and RTP source (`udp://`, `rtp://`)**
1. `Action::UdpBind` asks the host for a datagram socket. In receive mode it names the local address and port, and for a multicast group the optional interface and optional source address (source-specific multicast); in connect mode (SRT) it names the remote peer. The host reports success with `Session::on_connected(conn)` and failure with `on_closed(conn, Some(error), now)`. Each `Session::on_data(conn, bytes)` carries exactly one datagram; boundaries are never merged or split by the host.
2. A plain UDP datagram must be a whole number of 188-byte TS packets, each starting with the sync byte. Anything else is dropped whole and counted as malformed, so one damaged datagram cannot desynchronise the demuxer.
3. An RTP datagram is parsed with `rtp-packet`; only payload type 33 (MP2T) is accepted, the rest is counted as malformed. A bounded reorder buffer releases payloads in sequence order, tolerates 16-bit sequence wrap, drops duplicates and packets that arrive after their slot was given up on, and declares a gap lost when the buffer would exceed its window or its oldest entry waited `reorder_wait_us`. Lost, reordered, duplicate and late counts are readable from `Session::datagram_stats()`. A change of SSRC, or a sequence jump beyond the window, flushes the buffer and emits `Event::Discontinuity`.
4. Silence is handled by the TS-over-HTTP policy: the start timeout from bind to the first sample, `Starved(NoData)` and one `Stalled` per second after `stall_notice_ms`, `Live` on the next datagram, and a rebind (a new `UdpBind` after `Close`) after `stall_reconnect_ms`. A bind failure backs off as a TS-HTTP connect failure does. Timers are driven only by `poll(now_us)` and `WakeAt`.
5. Output for the same TS is identical for 1-byte-per-datagram-boundary changes: replaying a capture as 1316-byte datagrams, as 188-byte datagrams, and as RTP with shuffled neighbours (inside the window) gives the same `Tracks`/`Sample` sequence as the TS-over-HTTP source reading the same file.
6. `close()` emits one `Close` per live socket. Host-side socket options (`SO_RCVBUF`, `SO_REUSEADDR`/`SO_REUSEPORT`, the multicast interface, `IP_ADD_SOURCE_MEMBERSHIP`) are described for `socket2`; the crate never calls them.

**SRT caller (`srt://host:port`)**
1. The session drives `srt-runtime`'s existing caller handshake, ARQ receiver and TSBPD scheduler; no SRT packet format, handshake step, loss rule or timing rule is reimplemented in this crate. The host provides one connected UDP socket (`UdpBind` in connect mode), sends every `Action::Send` as one datagram and feeds every received datagram to `on_data`.
2. Configuration: TSBPD latency, an optional stream id, an optional passphrase with the AES key length, and an entropy seed. The seed is 32 bytes from the host's CSPRNG; the session derives each connection's socket id, initial sequence number, salt and stream key from it with HMAC-SHA-256 and a per-attempt counter, so a reconnect never reuses a value. A session for an `srt://` URL without a seed is `Error::InvalidConfig`.
3. Handshake: induction and conclusion are retransmitted by `poll`; a rejection becomes a non-fatal `Event::Error` that names the peer's reason, then the connect-failure backoff; a handshake that does not finish inside its budget is a connect failure. The source reports `Establishing` at bind and `Live` at the first sample.
4. Data: packets are staged, acknowledged and NAKed by the ARQ receiver, released in order at their play time by TSBPD (with too-late drop only when negotiated), and decrypted with the negotiated key when a passphrase is set. Each released payload goes through the same TS alignment as UDP and into the TS source.
5. A wrong passphrase is rejected by the handshake; a packet encrypted under a key the session does not hold is dropped and counted (`undecryptable`), never delivered as garbage.
6. Liveness: keepalive after one idle second, peer-idle expiry after `peer_idle_ms` (default 5000), and a peer SHUTDOWN end the connection. A connection that had played reconnects at once with fresh entropy and emits `Discontinuity`; one that never played backs off. `close()` sends one SHUTDOWN datagram, then `Close`.
7. Verified against `srt-runtime`'s own listener in-process (clear and encrypted, loss, reorder, duplicate, stall, reconnect, rejection, wrong passphrase) and against libsrt's `srt-live-transmit` when installed (loud skip when absent, hard failure under `REQUIRE_LIBSRT=1`).

**Client identity (mTLS)**
1. `Config::client_identity` holds either an opaque host handle (the host's keychain or enclave owns the key; the session never sees key material) or a PEM or DER certificate chain with its private key. Malformed material is `Error::InvalidConfig` at `open`, checked structurally (a chain of well-formed DER certificates and one private key, not a full path validation).
2. The identity rides on every action the host performs over TLS: `HttpRequest` for an `https` URL (TS-HTTP, HLS manifests, segments and keys, DASH) and `TcpConnect` with `tls: true` (RTSPS). It is attached only for the session's own origin unless `client_identity_scope` is `AnyHttps`, so a redirect to a foreign origin never offers it by default. Plain `http` requests carry none.
3. The key bytes are held once (behind an `Arc`), are redacted in every `Debug`, and are never copied into events.
4. Tested end to end against a local rustls server that requires client authentication: with the identity the capture plays, without it the TLS handshake fails and the session reports a connect failure with backoff, and a certificate from the wrong CA is refused. The test host performs the TLS exactly as the action instructs.
