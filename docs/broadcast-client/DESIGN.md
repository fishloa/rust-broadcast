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

## BC-2 sketch (not for BC-1)

`buffering-runtime::BufferController`: `on_chunk(arrival_us, bytes, pcr)` from `on_data` (+ PCR from
`DemuxEvent::ClockReference`), `on_track_media` from Samples (withheld before the first video random-access
point, per its caller contract), `on_tick(now, playback)` from a new player input
`Session::on_playback_position(now_ms, pts_27mhz)`; its `StartPlaying/Pause/Resume` become `Event::Buffer`.

## Finite resources (VOD)

A 206 response to our own plain GET is finite too. Redirect/auth layers are unchanged; headers are
forwarded to tshttp-runtime for finite detection. `Event::Finite` is mapped from tshttp, and
`Event::Ended` becomes `Status(Ended)`. The session does not reconnect after Ended; `Session::reconnect()`
restarts the session (attempt 0). Truncated transfers (EOF before all expected bytes) reconnect.

## BC-3: HLS source (hls-runtime::HlsClient inside the session)

Scope split: **BC-3a** single media playlist and master playlist with the policy-chosen variant (muxed audio), TS and fMP4 segments, VOD and live. **BC-3b** separate audio/subtitle renditions as selectable tracks (later, own story). No Event::Buffer for HLS in BC-3a (segment delivery is bursty by nature; pacing is the player's media-clock business, like VOD).

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
