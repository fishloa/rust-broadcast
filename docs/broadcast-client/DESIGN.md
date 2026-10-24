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
to tshttp `on_error`, `None` to `on_eof`. `close()` is terminal: it drops every queued action except
`Close`, adds one `Close` per live conn (never twice for the same conn), then `poll`/`next_event` return
None.

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

A 206 response to our own plain GET is finite too. Redirect/auth layers are unchanged; headers are
forwarded to tshttp-runtime for finite detection. `Event::Finite` is mapped from tshttp, and
`Event::Ended` becomes `Status(Ended)`. The session does not reconnect after Ended; `Session::reconnect()`
restarts the session (attempt 0). Truncated transfers (EOF before all expected bytes) reconnect.

### BC-2 corrections from the player's baseline (zenith-tv review; binding)

- (a) Video media end is NOT the last sample's pts+duration: it is `maxPTS + duration of the current (latest kept) sample`, where `maxPTS` = the max PTS (presentation order) over KEPT video samples seen so far since the join point (B-frames: the last sample in decode order can have a lower PTS). Reference: PlayerController.swift l.759-762 (`lastVideoPTS = max(lastVideoPTS, s.pts)`; `pendingVideoEnd = videoTime(lastVideoPTS + s.duration)`). Only samples from the join point on count; `Drop(_)` and pre-join samples never move it. Reported once per chunk (the latest pending value).
- (b) Audio media end = max over the RENDERED audio track of (pts + duration), reported only once it is past the video join point's PTS (`end > join_pts`, l.916). When the rendered audio track changes (set_rendered_tracks with a different audio), forget the old track's end and (re)register the new one. Audio samples of non-rendered tracks never count.
- (c) `Session::rejoin()`: decoder reset / player restart WITHOUT a new timeline: re-arms every video track's `LiveJoin` (`rejoin()`), clears pending ends and maxPTS/join state, does NOT hint `PcrDiscontinuity` and does NOT reset the controller. On a new timeline the player calls `rejoin()` AND `hint(PcrDiscontinuity)`. The session's own automatic PcrDiscontinuity (demuxer `Discontinuity`) does both.

## BC-3: HLS source (hls-runtime::HlsClient inside the session)

Scope split: **BC-3a** single media playlist and master playlist with the policy-chosen variant (muxed audio), TS and fMP4 segments, VOD and live. **BC-3b** separate audio renditions as selectable tracks (built, see "BC-3b renditions"; subtitle renditions remain later). No Event::Buffer for HLS in BC-3a; HLS and finite TS get the BC-5 playout gate (below).

- `SourceKind::Hls`; `Auto` picks Hls when the URL path ends with `.m3u8`/`.m3u` (ASCII case-insensitive), else TsHttp.
- Adapter `source/hls.rs` over `hls_runtime::client::HlsClient` (default features, no tokio):
  - `Action::FetchPlaylist{..}` -> `HttpRequest` GET to `action.playlist_request_url()` (blocking-reload query appended by hls-runtime), conn tagged `Playlist`. `Action::FetchResource{id,url,byte_range}` -> `HttpRequest{.., byte_range}` tagged `Resource(id)`. Up to the number of actions hls-runtime emits may be in flight at once (prefetch): the session keeps a conn table `ConnId -> purpose`.
  - Bodies accumulate per conn across `on_data` (bounded: `Config::max_playlist_bytes` 4 MiB, `max_segment_bytes` 64 MiB; exceeding = Close + failure) and are delivered on `on_closed(None)`: `on_playlist(bytes)` / `on_resource(id, bytes)`. A `HlsClient` parse error is a failure of that request.
  - `Action::WaitMs(ms)` -> `WakeAt(now_us + ms*1000)`; `HlsClient::next_wait()` converts the same way (the core takes `std::time::Instant` only in `poll_timeout`; the session never calls that, it computes deadlines from `now_us`).
  - Output mapping: `Master(sel)` -> `Event::Variants(MasterSelection)` (new event, variants+chosen+renditions as in hls-runtime); `Tracks` -> `Event::Tracks`; `Init` dropped (not surfaced); `Samples{track_id, samples}` -> one `Event::Sample{track, sample, join: None}` per sample, in order; `DisplayChanged` -> `Event::DisplayChanged`; `Discontinuity` -> `Event::Discontinuity`; `EndOfStream` -> `Status::Ended`.
  - Status: `Establishing` until the first Sample, then `Live` once; `Status::Ended` for ENDLIST VOD at end of stream.
- New inputs: `Config::variant_policy: hls_runtime::client::VariantPolicy`; `Session::select_variant(index)`; both delegate to HlsClient.
- Retry policy (the core has none, "the caller drives retry cadence"): a failed request (transport error, non-2xx other than 401/407/3xx handled by the auth/redirect layers, truncated or oversized body, parse error) calls `HlsClient::on_error(id)` and schedules the next `poll` retry with the same table as TS-HTTP: backoff 2, 4, 8 s then 10 s forever (`Config::hls_backoff_ms`, `hls_backoff_cap_ms`), attempt counter resets on any successful playlist or resource. 404/410 on the MAIN (first) playlist: `Status::NotFound` per occurrence + same backoff; 404/410 on a segment: counted as a failure for that resource only (no NotFound status). While in backoff `Status::Reconnecting{attempt}` is emitted on each scheduled retry. Never gives up on live; the only terminal states are `Ended` (ENDLIST) and `close()`.
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
  parsing is `sdp-types`, fmtp/rtpmap decoding is `transmux::rtp_sdp`. Per-track SETUP URIs are
  `Url::join(a=control)` against the request URI (aggregate/absent control = the request URI). Media other
  than H.264 video and AAC audio, a missing fmtp, an SDP without media or unparsable SDP are a failure.
- Media: interleaved frames are routed by channel: the track's RTP channel to `push`, RTP channel + 1 to the
  depayloader's `push_rtcp` (sender reports; no receiver reports are sent). `Event::Tracks` before the first
  `Sample` after PLAY of every connection, `Event::Sample { join: None }` per sample, `Status::Establishing`
  on connect, `Status::Live` once at the first sample. A malformed RTP frame is counted
  (`Session::rtp_malformed_count`) and skipped; `RtpLossEvent::SequenceGap`/`DamagedAccessUnit` are counted
  (`Session::rtp_loss_count`) and the stream continues (`NonMonotonicTimestamp` is B-frame reordering, not
  loss, and is not counted). When a connection ends after PLAY the depayloader is flushed so the last access
  units are delivered.
- Keepalive: `OPTIONS` on the request URI at half the `Session` header's `timeout` clamped to 5 s..3600 s,
  default 30 s, armed at the PLAY reply and re-armed at each send, driven by `poll(now_us)`. Any reply to it
  is ignored (405/501 are common). `close()` sends `TEARDOWN` (only when a session id exists) and then
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

Inputs (all from the session): per-rendered-track media end (27 MHz, the BC-2 definition: video end = max PTS of kept video samples since the origin + the latest sample's duration, audio end = max of pts + duration; finite TS skips `JoinDecision::Drop` samples, HLS has no `LiveJoin`), `on_playback_position(now_us, Option<pts_27mhz>)`, `set_rendered_tracks` (default: first video + first audio announced), `rejoin()`, `Event::Discontinuity` (and a demuxer discontinuity), a variant switch, and end of stream (`Status::Ended`; for HLS only once the main and any audio rendition playlist ended). The gate sees merged track ids, so the BC-3b audio rendition (id + 1000) is an ordinary audio track in the minimum.

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
- `Config::max_queued_events` (default 100 000): when the host does not drain `next_event`, the oldest queued
  `Sample`s are dropped and counted by `Session::dropped_events()`; every other event is kept. The action
  queue is not capped: dropping a `Send`, `HttpRequest` or `Close` would corrupt the protocol state, and every
  callback adds a bounded number of actions.
- Cookies (RFC 6265bis): a `Secure` cookie from an `http` origin is rejected, `__Secure-` needs `Secure`,
  `__Host-` needs `Secure`, `Path=/` and no `Domain` (prefixes match ASCII case-insensitively), a `Path` over
  1024 bytes is rejected.
- `auth` and `redirect` are `#[doc(hidden)]` (the integration tests need them); everything else internal is
  crate-private.
