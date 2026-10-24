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
- `Config::buffer: buffering_runtime::ControllerConfig` (default = the crate default) and `Session::open_with_memory(.., Option<StallMemorySnapshot>)`/`Session::stall_memory() -> StallMemorySnapshot` so memory survives sessions (zenith seeds it today).

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

Default set: `Session::default_rendered_tracks() -> (Option<TrackId>, Option<TrackId>)` = the lowest `TrackId` of the video kind and of the audio kind among the announced tracks (not first-added: track ids follow resolution order, a host assumes lowest-id). Until `set_rendered_tracks` is called the controller follows it, re-targeting (unregister old, register + seed new) when a lower id is announced. `Event::Tracks` is a tuple variant, so the default is exposed by the getter only. The BC-5 playout gate (HLS, finite TS) keeps its own first-announced default.

## BC-3: HLS source (hls-runtime::HlsClient inside the session)

Scope split: **BC-3a** single media playlist and master playlist with the policy-chosen variant (muxed audio), TS and fMP4 segments, VOD and live. **BC-3b** separate audio renditions as selectable tracks (built, see "BC-3b renditions"; subtitle renditions remain later). No Event::Buffer for HLS in BC-3a; HLS and finite TS get the BC-5 playout gate (below).

- `SourceKind::Hls`; `Auto` picks Hls when the URL path ends with `.m3u8`/`.m3u` (ASCII case-insensitive), else TsHttp.
- Adapter `source/hls.rs` over `hls_runtime::client::HlsClient` (default features, no tokio):
  - `Action::FetchPlaylist{..}` -> `HttpRequest` GET to `action.playlist_request_url()` (blocking-reload query appended by hls-runtime), conn tagged `Playlist`. `Action::FetchResource{id,url,byte_range}` -> `HttpRequest{.., byte_range}` tagged `Resource(id)`. The session keeps a conn table `ConnId -> purpose`. In-flight segment and part requests are capped at `Config::hls_max_inflight_segments` (default 4, `0` means 1) per playlist adapter: the cap counts every segment/part that was issued and not yet delivered to the core (in flight, or completed and held for in-order delivery, or failed and awaiting retry), so held bodies are bounded by `cap * max_segment_bytes`; the rest of hls-runtime's actions stay queued and are released as responses complete, delivery to the core stays in order. A retry of a segment already inside the window is never blocked by the cap (a failed segment cannot deadlock the window); playlist, key and init requests are not counted and never wait behind segments (queued actions leave in the order the core emitted them among those allowed to go). Request timeouts are tracked in a deadline heap with lazy deletion, so `poll` is O(log n) per expiry, not a scan of every request.
  - Bodies accumulate per conn across `on_data` (bounded: `Config::max_playlist_bytes` 4 MiB, `max_segment_bytes` 64 MiB; exceeding = Close + failure) and are delivered on `on_closed(None)`: `on_playlist(bytes)` / `on_resource(id, bytes)`. A `HlsClient` parse error is a failure of that request.
  - `Action::WaitMs(ms)` -> `WakeAt(now_us + ms*1000)`; `HlsClient::next_wait()` converts the same way (the core takes `std::time::Instant` only in `poll_timeout`; the session never calls that, it computes deadlines from `now_us`).
  - Output mapping: `Master(sel)` -> `Event::Variants(MasterSelection)` (new event, variants+chosen+renditions as in hls-runtime); `Tracks` -> `Event::Tracks`; `Init` dropped (not surfaced); `Samples{track_id, samples}` -> one `Event::Sample{track, sample, join}` per sample, in order (`join` set by the session, see Audit round 5 F5); `DisplayChanged` -> `Event::DisplayChanged`; `Discontinuity` -> `Event::Discontinuity`; `EndOfStream` -> `Status::Ended`.
  - Status: `Establishing` until the first Sample, then `Live` once; `Status::Ended` for ENDLIST VOD at end of stream.
- New inputs: `Config::variant_policy: hls_runtime::client::VariantPolicy`; `Session::select_variant(index)`; both delegate to HlsClient.
- Retry policy (the core has none, "the caller drives retry cadence"): a failed request (transport error, non-2xx other than 401/407/3xx handled by the auth/redirect layers, truncated or oversized body, parse error) calls `HlsClient::on_error(id)` and schedules the next `poll` retry with the same table as TS-HTTP: backoff 2, 4, 8 s then 10 s forever (`Config::hls_backoff_ms`, `hls_backoff_cap_ms`), attempt counter resets on any successful playlist or resource. 404/410 on the MAIN (first) playlist: `Status::NotFound` per occurrence + same backoff; 404/410 on a segment: counted as a failure for that resource only (no NotFound status). While in backoff `Status::Reconnecting{attempt}` is emitted on each scheduled retry. Never gives up on live; the only terminal states are `Ended` (ENDLIST) and `close()`. Every `hls_backoff_ms` entry and `hls_backoff_cap_ms` is raised to at least 1 ms, so a zero never produces a burst of retries at one instant.
- Auth/redirect layers apply to EVERY request exactly as for TS-HTTP: credentials scoped to the session origin (playlist origin) only; segments on other origins (CDN) carry no Authorization; per-request redirects (each request has its own redirect counter).
- Stall handling: no bytes for `Config::ts_http.stall_*` does NOT apply; a request in flight longer than `Config::hls_request_timeout_us` (default 20 s, playlists) / `hls_segment_timeout_us` (default 60 s) is aborted (`Close`) and counts as a failure.

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
- Retry: every failure bumps one attempt counter, emits `Status::Reconnecting{attempt}`, emits `WakeAt(now + backoff)` and gates the core queue until then. A failed playlist is refetched (same URL) at the wake time; a failed resource relies on the next playlist reload (the live reload chain, or a refetch scheduled when the chain is dead, e.g. VOD) to be re-requested after `HlsClient::on_error`. The counter resets when a playlist or resource is accepted by the core.
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

- `src/cookies.rs`: host-only jar scoped to the session origin; `Domain` and `Expires` ignored; default-path and path-match per RFC 6265 5.1.4; `Secure` only on https; `Max-Age` relative to the `now_us` of the response (<= 0 deletes, expiry is exclusive: gone at `stored_at + max_age`); same name+path replaces in place; `Cookie` header ordered longest path first, then storage order; at most 50 cookies (oldest evicted) and 4096 bytes of name+value per cookie. `Set-Cookie` lines are parsed with the `cookie` crate; every `Set-Cookie` line of any status (401 and 3xx included) is processed, but only for responses of the session origin.
- The jar lives in `AuthState`, the single layer every request passes through (`request_headers(url, now_us)`), so the TS-HTTP and HLS adapters share it: playlists, segments, reconnects and in-origin redirects carry the cookies; requests to another origin carry no cookie, no extra header and no `Authorization`.
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
  describe H.264 or AAC is the one played, so a `rtx` payload type listed first does not hide the real one. An absolute control whose
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
- Keepalive: `OPTIONS` on the aggregate control URL (else the request URI) at half the `Session` header's `timeout` clamped to 5 s..3600 s,
  default 30 s, armed at the PLAY reply and re-armed at each send, driven by `poll(now_us)`. Any reply to it
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

## BC-5 playout gate

`Event::Buffer(Start|Pause|Resume)` for the sources the PDV controller does not serve: HLS live, HLS VOD (`EXT-X-ENDLIST` seen) and finite TS (`Event::Finite`). Module `src/playout.rs`; live TS-over-HTTP keeps the BC-2 `buffering-runtime` path unchanged (`Config::buffer`). The gate is deterministic and lead-based: it does not measure jitter, so its events depend only on the delivered samples, the end of stream and the reported playback position, never on chunking or wall-clock time.

Inputs (all from the session): per-rendered-track media end (27 MHz, the BC-2 definition: video end = max PTS of kept video samples since the origin + the latest sample's duration, audio end = max of pts + duration; finite TS and HLS skip `JoinDecision::Drop` samples), `on_playback_position(now_us, Option<pts_27mhz>)`, `set_rendered_tracks` (default: first video + first audio announced), `rejoin()`, `Event::Discontinuity` (and a demuxer discontinuity), a variant switch, and end of stream (`Status::Ended`; for HLS only once the main and any audio rendition playlist ended). The gate sees merged track ids, so the BC-3b audio rendition (id + 1000) is an ordinary audio track in the minimum.

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
  `Config::hls_max_inflight_segments` segment/part `HttpRequest`s are outstanding per adapter (plus the
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
- **Host contract.** poll until `None` and drain after every callback; live TS needs
  `on_playback_position` every display tick; `Start.at`/`Resume.at` are the 27 MHz clock the player
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

1. every PMT-declared elementary stream has resolved: a shadow PAT/PMT reader (`source/ts_http_hold.rs`:
   `mpeg_ts::resync::TsResync` + `dvb_si::demux::SiDemux`, fed only until release) learns the programmes the PAT
   lists, the elementary PIDs their PMTs declare (all programmes: the demuxer demuxes all of them), and a stream
   counts as resolved when the demuxer reports `TrackAdded`/`TrackUpdated` with that `source_pid` or a
   `TrackAbandoned` with that PID; the PAT must also have had every listed PMT seen;
2. a limit, counted from the first media of the connection: `HOLD_QUIET_INPUT_BYTES` (512 KiB of input with no
   further stream resolving: the scrambled `csa/csa2-plain.ts` leaves declared streams unresolved forever and
   round-5 F2's liveness test needs media within 0.56 MB of it), `HOLD_MAX_INPUT_BYTES` (4 MiB of input),
   `HOLD_MAX_MEDIA_US` (3 s of media, the largest per-track pts span among held samples), or `HOLD_MAX_WALL_US`
   (3 s of caller clock, armed through `next_deadline_us`/`WakeAt`, so a source that goes quiet mid-hold still
   releases);
   The limits are `Config::tracks_hold_quiet_input_bytes`, `tracks_hold_max_input_bytes`,
   `tracks_hold_max_media_us` and `tracks_hold_max_wall_us`; the values above are their defaults.
   A zero wall or media limit disables the hold (tracks are announced as each stream resolves, the
   pre-round-5 behaviour). A latency-sensitive host (first frame within 2.5 s) should lower the wall
   limit, for example to 500 ms, since a declared stream that never sends costs the full wall limit.
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
`france2.ts`) is released at the limit with the streams that did resolve. Tracks that resolve after the release
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
