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

A 206 response to our own plain GET is finite too. Redirect/auth layers are unchanged; headers are
forwarded to tshttp-runtime for finite detection. `Event::Finite` is mapped from tshttp, and
`Event::Ended` becomes `Status(Ended)`. The session does not reconnect after Ended; `Session::reconnect()`
restarts the session (attempt 0). Truncated transfers (EOF before all expected bytes) reconnect.

### BC-2 corrections from the player's baseline (zenith-tv review; binding)

- (a) Video media end is NOT the last sample's pts+duration: it is `maxPTS + duration of the current (latest kept) sample`, where `maxPTS` = the max PTS (presentation order) over KEPT video samples seen so far since the join point (B-frames: the last sample in decode order can have a lower PTS). Reference: PlayerController.swift l.759-762 (`lastVideoPTS = max(lastVideoPTS, s.pts)`; `pendingVideoEnd = videoTime(lastVideoPTS + s.duration)`). Only samples from the join point on count; `Drop(_)` and pre-join samples never move it. Reported once per chunk (the latest pending value).
- (b) Audio media end = max over the RENDERED audio track of (pts + duration), reported only once it is past the video join point's PTS (`end > join_pts`, l.916). When the rendered audio track changes (set_rendered_tracks with a different audio), forget the old track's end and (re)register the new one. Audio samples of non-rendered tracks never count.
- (c) `Session::rejoin()`: decoder reset / player restart WITHOUT a new timeline: re-arms every video track's `LiveJoin` (`rejoin()`), clears pending ends and maxPTS/join state, does NOT hint `PcrDiscontinuity` and does NOT reset the controller. On a new timeline the player calls `rejoin()` AND `hint(PcrDiscontinuity)`. The session's own automatic PcrDiscontinuity (demuxer `Discontinuity`) does both.

## BC-3: HLS source (hls-runtime::HlsClient inside the session)

Scope split: **BC-3a** single media playlist and master playlist with the policy-chosen variant (muxed audio), TS and fMP4 segments, VOD and live. **BC-3b** separate audio renditions as selectable tracks (built, see "BC-3b renditions"; subtitle renditions remain later). No Event::Buffer for HLS in BC-3a (segment delivery is bursty by nature; pacing is the player's media-clock business, like VOD).

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
- VOD (a `Finite` resource): the controller is not created (no live-edge decisions); `Event::Buffer` is never emitted; pacing is the player's media-clock business.

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
