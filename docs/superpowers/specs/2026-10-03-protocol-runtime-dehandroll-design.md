# Protocol & runtime de-hand-roll — design

Date: 2026-10-03 · Status: approved in conversation, awaiting written-spec review

## 1. Intent

**Outcome.** No generic protocol or format in the workspace is parsed, built,
framed, scheduled or served by hand-rolled code where an established crate or
runtime facility does the job. That covers HTTP, RTSP header framing, URLs,
SDP, ICE candidate lines, base64, hex, dates and durations, Prometheus
exposition, and the IO, event-loop and threading machinery around them.
Real defects found while surveying this are fixed in the same work. All
outdated dependencies are taken.

**Why.** Hand-rolled protocol code has repeatedly been where bugs lived:
- quadratic RTSP reparse
- slow-loris HTTP readers
- relative URLs that mishandle `..`
- WHIP timers that never fire
- accept starvation
- tasks that leak a bound port

The owner's rule, set by the XML migration (`30c9fa46`, quick-xml only), now
applies to everything generic.

**Success criteria.**
1. Every site in the inventory (§3) is replaced, or is listed in §9
   (documented exceptions) with its reason.
2. The full 14-step gate (`.delegate/gate.sh`) passes and CI is green on
   `main` after each wave.
3. Wire and serialised output (MPD, playlists, SDP, HTTP response headers,
   metrics) is byte-identical to the pre-change goldens, except for differences
   listed in the CHANGELOG with an example each.
4. Every fixed defect has a regression test that is shown to fail against the
   old code (revert-check evidence recorded).
5. No new test relies on wall-clock sleeps; every test touched in SP7 passes
   20 consecutive runs.

**Owner decisions (verbatim intent).**
- Q1: `no_std` may be dropped. Q2 scope: **only in the crates this work
  touches** (transmux, hls-runtime, webrtc-runtime core, timed-metadata,
  scte35-splice, broadcast-common as needed). Pure parser crates (dvb-si,
  mpeg-ts, mpeg-pes, dvb-csa, …) stay `no_std`.
- Take **all** dependency bumps.
- Split SP0–SP7 approved. **Approach A:** design by concern (this spec), and
  execute by crate cluster (§7).
- SP3: URL references with no absolute base use option (a), a `file://` /
  synthetic base with that base stripped from the result.
- SP4: fmtp parameter lists stay codec logic. Link header uses option (a), a
  documented exception.
- SP5: dates and durations use `jiff`.
- Event loops and threading models are in scope ("sort that at the same
  time").

Out of scope: XML (done); product formats that are a crate's own job (M3U8 in
broadcast-hls, WebVTT/SRT writers, TTML time expressions, DASH/Smooth URL
templates, Smooth request-path templates); atsc3/atsc3-route (abandoned);
migrating the existing optional `chrono` features (public API, unchanged).

## 2. Constraints

- MSRV 1.95.0; committed `Cargo.lock`, always `--locked`. A dependency add or
  bump may change only the intended lock entries; restore anything else with
  `cargo update -p <pkg> --precise <old>`.
- Every new or bumped crate supports MSRV 1.95: verified for all 34 bumps (max
  1.89) and for the new crates (`tokio-util` 1.71, `socket2` 1.70, `backon`
  1.85, `parking_lot` 1.71, `lru` 1.85, `jiff` 1.70, `hex`, `base64`,
  `arc-swap`, `wait-timeout`).
- No Co-Authored-By or Claude-Session trailers on commits.
- Nothing is tagged or published without the owner's explicit sign-off.
- Epoch purity: if a bumped dependency's types appear in a crate's public API,
  that crate takes a major-class version change. Each wave records this in
  `.delegate/release-versions.txt`.

## 3. Inventory (source of truth for "every site")

Line numbers are from `main` at `30c9fa46` and are indicative; the plan
re-greps.

**HTTP.**
- multimux `webrtc_http.rs` (request reader, slot cap); `source/whip.rs` and
  `output/whep.rs` (accept loops, hand-built responses including CORS, 401,
  409, 201).
- media-doctor `src/bin/media-doctor.rs` (thread-per-connection metrics
  server, head reader) and `watch.rs` (`render_prometheus`).
- broadcast-auth `server.rs` (WWW-Authenticate render, Authorization/Digest
  field parse, digest-uri match, hex/unhex), `authenticator.rs` (Bearer value),
  `signed_url.rs` (query build/parse, unescaped `kid`).
- webrtc-runtime `whep/server.rs`, `whip/server.rs`, `whep/player.rs`,
  `whip/client.rs` (ETag, If-Match, Content-Type match, Retry-After, Location,
  Bearer).
- hls-runtime `client/tokio_client.rs` (Range), `client/action.rs` + `url.rs`
  (query append).
- multimux `origin/mod.rs` (Cache-Control/CORS consts), `origin/limit.rs`
  (custom limit service, `_HLS_*` substring scan), `origin/admin.rs`
  (`RwLock<Router>` clone).
- transmux `uri.rs` CR/LF guard.

**RTSP.**
- rtsp-runtime `io.rs` (`has_header_end`, read loops, no timeouts),
  `transport.rs` (Transport header parse/build), `client.rs` (`stale=`
  scan, Session parse), `server.rs` (Session id).

**URL.**
- transmux `uri.rs` (full RFC 3986).
- hls-runtime `client/url.rs`.
- broadcast-auth digest-uri.
- multimux `redact.rs`, `push/srt.rs` (parse + percent-decode),
  `config.rs` host:port, `push/rtmp.rs` and `push/rtsp.rs` builds,
  `source/rtsp.rs` host:port/IPv6.
- rtmp-runtime `client.rs` tcUrl.
- webrtc-runtime `media/transport.rs` `stun:` build.

**SDP / ICE / Link.**
- transmux `rtp.rs` (`build_sdp*`, `sdp_video`, `sdp_audio`) and `rtp_sdp.rs`
  (fmtp/rtpmap, which stays per decision).
- multimux `source/whip.rs` and `output/whep.rs` (second-pass line scans,
  answer build, candidate lines), `push/rtsp.rs` (ANNOUNCE).
- webrtc-runtime `media/transport.rs` (fingerprint scan, parse and format) and
  `ice.rs` (Link, which is an exception).

**Encodings and time.**
- base64: transmux `rtp.rs`, scte35-splice `dvb_ta/stream_event.rs`.
- hex: transmux `rtp.rs`, `smooth_parse.rs`, `smooth.rs`, `dash.rs`,
  `sample_aes.rs`, `cli.rs`; timed-metadata `daterange.rs`; broadcast-auth
  `server.rs`; broadcast-common `hex.rs` (keep API, delegate).
- dates: transmux `cli.rs`; multimux `output/dash.rs`, `dvr.rs`,
  `source/dash_pull.rs`; timed-metadata `anchor.rs`; hls-runtime
  `client/engine.rs`.
- durations: transmux `dash_parse.rs` (parse), `dash.rs`; multimux
  `output/dash.rs` and `ll_dash.rs` (build).

**Runtime and concurrency.** These are items M1–M31 of the runtime inventory,
by crate:
- multimux: accept pump + 20 ms sleep-poll (rtmp, whip); accept loops with
  untracked spawns (whip, whep); HTTP reader; custom limit layer; four backoff
  implementations; three pull drive loops + 5 ms idle poll; six per-source
  read loops; UDP/multicast bind ×4 (with media-doctor and dvb-stream);
  WebRTC UDP loops with timer tied to recv timeout; `listen()` timeout/sleep
  fallback; file pacer; hand-rolled RTMP/RTSP push client loops; poison
  wrappers; Router clone.
- rtsp-runtime and rtmp-runtime: hand-rolled framing loops, no timeouts.
- srt-runtime: UDP demux pump (untracked, holds port), 2 ms tick per
  connection, handshakes only advance in `accept()`, per-datagram `to_vec`,
  fixed `MAX_DATAGRAM`.
- media-doctor: thread-per-connection server.
- dvb-stream: `TsFramer`, `UdpReader`.
- media-plane: single `Mutex<TrunkState>`, Condvar back-pressure, CAS waiter
  cap.
- dvb-ci-runtime: `libc::poll`.
- broadcast-auth: BTreeMap LRU.

**Defects** (each needs a regression test that fails before the fix):
1. WHIP `handle_timeout` only on read-timeout, after which the session ends.
2. RTMP/WHIP accept starvation under steady reads.
3. Untracked accept pumps (rtmp, whip, whep, srt listener); the SRT pump keeps
   its UDP port bound after drop.
4. No timeouts in rtsp-runtime `io.rs`, rtmp-runtime `io.rs`, multimux
   `push/rtsp.rs`; the RTSP source write has no timeout.
5. Push connect and push backoff sleep ignore cancellation; `acquire_owned`
   in whep accept ignores cancel; hls_pull `WaitMs` blocks join servicing.
6. Signed-URL `kid` not percent-encoded.
7. hls-runtime relative URL resolution mishandles `..`.
8. tcUrl/RTSP URL builds break on IPv6 hosts.

**Test harness.**
- Reserve-then-rebind ports (multimux tests `admin_api`, `dispatch_ingest`,
  `file_route`, `smooth_oracle`, `ts_hls_oracle`, `whep_egress`,
  `whip_ingest`).
- About 40 sleep-based waits.
- `bounded.rs` ×3 (byte-identical).
- Hand-rolled HTTP test origins (hls-runtime, media-doctor).
- multimux `tests/rtsp_ingest.rs` RTSP test server.
- webrtc-runtime `examples/whip_media_smoke.rs`.

## 4. Design by sub-project

### SP0 — dependency bumps
One commit per group: (1) semver-compatible `cargo update`; (2) RustCrypto
(aes 0.9, ctr 0.10, cbc 0.2, cipher 0.5, hmac 0.13, sha1/sha2/md-5 0.11,
pbkdf2 0.13, aes-kw 0.3); (3) rtc-* 0.21; (4) base64 0.23; (5) criterion 0.8;
(6) roxmltree 0.21 (atsc3). axum/tower-http/reqwest go with SP2, sdp-types
with SP4. Proof: the full gate, the crypto known-answer tests and oracle
fixtures (`transmux/tests/fixtures/ORACLES.md`, SRT key-wrap/PBKDF2 vectors),
libsrt interop, and the WebRTC DTLS fingerprint and loopback tests, all
unweakened.

### SP1 — runtime foundations (the shared IO pattern)
1. **Framing.** Each stream adapter is a `tokio_util::codec::Framed` whose
   `Decoder`/`Encoder` delegate to the existing sans-IO core.
   - Applies to the rtsp-runtime client/server and rtmp-runtime server, plus
     the new rtmp client adapter from SP6.
   - dvb-stream: `TsFramer` becomes a TS `Decoder`, and `UdpReader` becomes
     `UdpFramed`.
   - Datagram paths use `UdpFramed`.
   - The sans-IO cores keep their APIs. rtmp's `pending_write` cancel-safety
     code is deleted, because `FramedWrite`'s sink buffer is cancel-safe.
2. **Next-deadline scheduling.**
   - Each timer-bearing core exposes `fn poll_timeout(&self) -> Option<Instant>`:
     webrtc `MediaTransport`, the SRT connection/listener, the rtsp
     `ClientSession` keepalive, and the hls `HlsClient`.
   - Adapters `select!` over: inbound frame, outbound queue,
     `sleep_until(deadline)` and `cancel.cancelled()`.
   - Fixed ticks and sleep-polls are removed: the SRT 2 ms tick, the WHEP
     10 ms poll, multimux's 20 ms accept poll and 5 ms idle poll, and the hls
     10 ms defensive sleep.
3. **Timeouts.** Every adapter takes a config struct with `connect`,
   `handshake`, `read_idle` and `write`, all `Duration` with documented
   defaults. No awaited IO is unbounded.
4. **Tasks and shutdown.** Every spawn is owned by a `JoinSet` or
   `tokio_util::task::TaskTracker`; no `JoinHandle` is dropped. Shutdown uses
   `tokio_util::sync::CancellationToken` only. multimux's public
   `watch<bool>` shutdown (`registry.rs`) is replaced by the token, which is
   breaking.
5. **Retry and reconnect.** `backon` with jitter. One `ReconnectPolicy` maps to
   a `backon` builder and replaces `supervisor::Backoff`, `ReconnectEngine`
   timing, `file_reader` retry, `hls_pull` retry and the hls-runtime
   `TokioClient` retry. Permanent-failure classification stays as domain
   logic.
6. **UDP/multicast.** `socket2` is called directly at each bind site, with
   configurable `SO_RCVBUF`, `SO_REUSEADDR` and the multicast interface. There
   is no shared wrapper crate.
7. **Defects 1–5 fixed** with regression tests (§3).
8. **Tests.** Timer, deadline and timeout behaviour is tested with
   `#[tokio::test(start_paused = true)]` and `time::advance`. Loopback interop
   tests stay.

### SP2 — HTTP
1. **WHIP/WHEP listeners move to `axum::serve`** with
   `with_graceful_shutdown(token)`. `webrtc_http.rs` is deleted.
   - tower layers: `ConcurrencyLimitLayer`, `RequestBodyLimitLayer` (64 KiB),
     `TimeoutLayer` (10 s), tower-http `CorsLayer`.
   - Listen addresses stay as configured.
   - Chunked request bodies are now accepted.
2. **webrtc-runtime WHIP/WHEP sans-IO state machines** use `http::{HeaderMap,
   HeaderValue, StatusCode}` and `headers` typed `ETag`, `IfMatch`,
   `ContentType`, `RetryAfter`, `Location`. The core becomes `std` and the
   change is breaking.
3. **media-doctor.**
   - `metrics` + `metrics-exporter-prometheus` serve `/metrics`.
   - The bin's server, head reader and atomic cap are deleted.
   - `WatchState` feeds gauges and counters.
   - `render_prometheus` is removed, which is breaking.
4. **broadcast-auth.**
   - Authorization is parsed with `http-auth`'s `ChallengeParser`. RFC 7235
     auth-params are shared with credentials, but **the plan's first task
     verifies this** against RFC 7616 credential examples.
   - WWW-Authenticate is rendered via `ChallengeRef`.
   - Fallback if verification fails: `headers` typed `Authorization` for
     Basic/Bearer, with the Digest gap escalated to the owner.
   - The RFC 7616 hash chain stays.
   - The nonce table uses `lru`.
   - Signed URLs use `url`/`form_urlencoded`, which fixes defect 6.
5. **multimux origin.**
   - `limit.rs` keeps only its 3-budget classification, composed from
     `tower::limit::GlobalConcurrencyLimitLayer`, `load_shed` and `timeout`.
   - Admin uses `arc-swap` for the Router.
   - `_HLS_*` is read through axum `Query`.
   - Cache-Control uses `headers::CacheControl`.
6. hls-runtime Range uses `headers::Range`.
7. Bumps: axum 0.8, tower-http 0.7, reqwest 0.13.
8. **Tests.**
   - Response-header goldens taken before the change.
   - 413/431/timeout/chunked tests.
   - The WHIP/WHEP loopback and WebRTC interop tests stay green.

### SP3 — URLs
1. The `url` crate is used everywhere in §3, URL row:
   - `Url::join` for HLS resolution (defect 7)
   - `set_username`/`set_password` for redaction
   - `Url` builders for tcUrl, RTSP control and `stun:` URLs (defect 8)
   - `query_pairs`/`query_pairs_mut` for SRT and HLS queries
   - `percent-encoding` only where `url` doesn't cover it
2. `host:port` strings use `SocketAddr` parsing or `tokio::net::lookup_host`.
3. **transmux `uri.rs`** is deleted.
   - BaseURL chains resolve with `Url::join` against the MPD's own URL: the
     source URL, or the file's `file://` URL.
   - For in-memory input with no location, a fixed synthetic base is used
     (`transmux-relative:///`), and the prefix is stripped from results that
     stay relative. The strip goes through one function, tested.
   - The RFC 3986 §5.4 example vectors (`uri.rs` 305–354) move to tests
     against the new path.
4. Tests: `..` in HLS, IPv6 in tcUrl/SRT/RTSP, redaction with
   percent-encoded credentials, and the RFC 3986 vectors.

### SP4 — SDP / ICE / Link
1. `sdp-types` 0.2 handles all SDP parsing and writing listed in §3.
   - WHIP/WHEP read candidates, ICE credentials, rtpmap, fmtp, rtcp-fb and
     media sections from `Session`'s typed API.
   - Answers, ANNOUNCE and transmux `build_sdp*` build a `Session` and call
     `Session::write`. transmux's RTP SDP is `std`-only.
   - The webrtc-runtime fingerprint is read from typed attributes.
2. ICE candidates are built with the `rtc-ice` candidate type plus marshal.
3. fmtp parameter content stays codec logic (owner decision), using `base64`
   and `hex` from SP5.
4. The Link header is a documented exception (§9).
5. Tests: SDP goldens taken before the change (byte-identical, or semantic
   equality with the difference listed); WHIP/WHEP and RTSP interop green.

### SP5 — encodings and time
1. `base64` replaces both hand-rolled codecs.
2. `hex` replaces the hex sites; `broadcast_common::hex` keeps its API and
   delegates, so it is not breaking.
3. `jiff` handles RFC 3339 / ISO 8601 timestamps (`Timestamp`) and
   `xs:duration` (`Span`, ISO 8601), replacing the three `civil_from_days`
   copies, the five ISO 8601 implementations and the duration parse/build.
   The `chrono` features are untouched.
4. Tests:
   - RFC 4648 vectors
   - RFC 3339 examples
   - XML Schema duration examples
   - existing MPD `availabilityStartTime`/`@duration`/`EXT-X-PROGRAM-DATE-TIME`
     output byte-identical to the goldens

### SP6 — concurrency structure
1. **multimux uses runtime-crate adapters.**
   - RTSP source and push go through rtsp-runtime's adapter.
   - RTMP push goes through a new rtmp-runtime client adapter.
   - The six source read loops become one generic ingest driver: a framed
     stream + timeouts + `IngestDriver::feed`/`advance_route`.
2. **One pull scheduler for HLS/DASH/Smooth.**
   - `buffer_unordered(MAX_INFLIGHT_FETCHES)` + `backon`, event-driven.
   - The plan first resolves `hls_pull.rs:7-20`'s reason for not using
     hls-runtime: either multimux drives the shared `HlsClient` core, or the
     reason is recorded as a documented exception.
3. **WebRTC session tasks own their `MediaTransport`.** There is no
   `Arc<tokio::Mutex<MediaTransport>>` across `send_to().await`.
4. **media-plane `Trunk`.**
   - First add a criterion contention benchmark (1 publisher, N cursors).
   - Split the lock per log only if the benchmark shows contention, and
     record the numbers before and after.
   - The Condvar back-pressure and CAS waiter cap become `parking_lot`
     Condvar + a `Semaphore`-equivalent only where they simplify; otherwise
     leave them and record why.
5. **SRT.**
   - The listener runs handshakes in a tracked background accept task.
   - Datagrams are passed as `Bytes`.
   - `MAX_DATAGRAM` is configurable.
6. `parking_lot` in multimux and media-plane; `lock.rs` poison wrappers are
   deleted.
7. dvb-ci-runtime: `libc::poll` is replaced by `rustix::event::poll`.

### SP7 — test harness
1. Bind port 0 and pass the bound listener into the code under test. Add
   `*_with_listener` constructors where they are missing, and delete the
   rebind-retry loops.
2. No fixed sleeps: tests wait on the real condition with a bounding
   `timeout`, or use paused time.
3. `bounded.rs` ×3 is replaced by `wait-timeout`.
4. Test servers:
   - HTTP test origins move to axum.
   - multimux `tests/rtsp_ingest.rs` moves to rtsp-runtime's server adapter.
   - The webrtc example moves to axum + tokio.
   - External oracles stay: mediamtx, libsrt, ffprobe, MP4Box, Bento4,
     mediastreamvalidator.
5. Every touched test passes 20 consecutive runs before merge.

## 5. Guards (keep it from coming back)

Each touched crate gets a lexical tripwire test, the same pattern as the XML
`no_dom_guard`. It scans `src/**/*.rs` outside `#[cfg(test)] mod` bodies, with
a reasoned allowlist, for these patterns:
- `"HTTP/1.`
- `"\r\n\r\n"`
- `find("://")`
- `strip_prefix("<scheme>://")`
- `"a=`/`"m=`/`"v=0` string building
- `civil_from_days`/`days_from_civil`
- a base64 alphabet literal
- `thread::sleep` / `sleep(` in non-test async code

The module doc states it is a tripwire and that review is the real control.

## 6. Testing & verification (every wave)

- Goldens are generated from **main before the wave** and committed, with a
  README giving the commit and command. Comparison is byte-for-byte.
- Every defect fix and every behavioural change has a test, revert-checked:
  the failing output against the old code is recorded in the wave report.
- Interop and oracle suites run unchanged: libsrt, mediamtx, ffprobe, MP4Box,
  Bento4, mediastreamvalidator, pycryptodome vectors.
- One adversarial reviewer per wave (correctness and tests), looping until
  clean. Then Claude runs `.delegate/gate-wt.sh` (14/14) on the final branch,
  merges, pushes, and confirms CI green.

## 7. Execution — by crate cluster (approach A)

| wave | cluster(s) | scope |
|---|---|---|
| W0 | workspace | SP0 groups 1–6 |
| W1 (parallel, ≤4 agents, one worktree each) | **R-low**: rtsp-runtime, rtmp-runtime, srt-runtime, webrtc-runtime, hls-runtime, dvb-stream | SP1 (framing, `poll_timeout`, timeouts, tasks, backon in hls, socket2 in dvb-stream), the new rtmp-runtime client adapter (SP6.1), SP2.2/2.6, SP3 (hls, rtmp tcUrl, webrtc stun), SP4 (webrtc fingerprint/ICE), SP5 (hls dates), SP6.5 (srt), SP7 for these crates, defects 4 (rtsp/rtmp), 7, 8 (rtmp) |
| | **T**: transmux, timed-metadata, scte35-splice, broadcast-common, broadcast-auth | SP2.4, SP3 (uri.rs, digest-uri), SP4 (transmux SDP), SP5 (all), defect 6 |
| | **P**: media-plane, media-doctor, dvb-ci-runtime | SP2.3, SP1.6 (media-doctor UDP), SP6.4/6.6/6.7, SP7 for these crates |
| W2 | **M**: multimux, multimux-cli | everything multimux: SP1–SP7 on top of W1's APIs, including the axum 0.8 / tower-http 0.7 / reqwest 0.13 bumps and defects 1, 2, 3, 4 (push/source), 5, 8 (multimux) |
| W3 | workspace | guards (§5) across crates, CLAUDE.md crate descriptions, the release-version recording, cleanup sweep re-running the §3 greps |

The sdp-types 0.2 bump lands in W1-T (transmux dev-dep) and in W1-R-low via
rtsp-runtime, and multimux follows in W2. Each W1 cluster must keep the
workspace building, so cross-crate API changes consumed by multimux land with
a minimal multimux compile fix, and the real multimux migration happens in W2.

## 8. Versioning (recorded per wave, published only with sign-off)

Breaking changes expected:
- multimux (shutdown token, axum 0.8 types)
- webrtc-runtime (std core, `http` types)
- media-doctor (`render_prometheus` removed; `check_dash_mpd` std already)
- rtsp-runtime and rtmp-runtime (adapter API: Framed + config)
- srt-runtime (listener and config)
- dvb-stream (stream constructors)
- hls-runtime (TokioClient config, std)
- transmux (std for SDP, `uri` module removed)
- broadcast-auth (signed URL query encoding)

broadcast-common is **not** breaking (`hex` delegates). Each wave updates
`.delegate/release-versions.txt` and runs
`tools/check-published-dep-consistency.py`.

## 9. Documented exceptions

1. **Link header (RFC 8288)**, webrtc-runtime `ice.rs`. No suitable crate
   exists:
   - `parse_link_header` 0.4.1 keys links by `rel`, which loses WHIP's multiple
     `ice-server` links.
   - `http-link` 1.0.1 writes unquoted `rel` plus a forced `anchor`, and has
     about 9k downloads (last release 2021).
   The parser and builder stay, with RFC 8288 + RFC 9725 test vectors and a
   fuzz target added.
2. **fmtp parameter lists**: codec payload-format logic, owned by transmux,
   alongside `sprop-parameter-sets`.
3. **Product formats** listed in §1 out-of-scope.
4. **RTSP `Transport` (RFC 2326 §12.39) and `Session` (§12.37) headers**,
   rtsp-runtime `rfc2326_lex.rs` / `transport.rs` / `session_header.rs`
   (OWNER-APPROVED exception, decision (c), 2026-10-03). `rtsp-types` 0.1.3 is
   exact-case, does not trim whitespace, silently drops case-variant
   parameters into an unknown-parameter map, and rejects a quoted `ssrc` or a
   malformed `timeout` (probe: `.delegate/rtsp-types-probe.txt`; write-up under
   "rtsp-types gaps" in `rtsp-runtime/docs/transport-header.md`, for a future
   upstream issue). rtsp-runtime therefore owns a full, spec-grounded parser and
   canonical serializer for these two headers, built on ONE RFC 2326 §15.1
   lexer; `rtsp-types` stays for message framing, the other headers and
   request/response building. The lexical guard allows exactly that lexer
   (line-pinned). Test vectors: every Transport/Session example in
   `docs/rfc2326.md`, the probe inputs, interop shapes, malformed-value errors,
   and a fuzz target (`rtsp_headers`) asserting the round-trip invariants.
5. **multimux redaction fallback** (`multimux/src/redact.rs`), the
   masking-only scrub for a URL-shaped string the `url` parser REJECTS. A
   crate cannot do this: the whole point is that the string is not a URL, so
   there is nothing to hand to a parser — `Url::parse` returns `Err` and the
   URL already failed to parse at connect time (the common case for the
   connect-time error messages this exists to sanitize). The fallback
   therefore scans the raw text for the `://` boundary and the `@`, and only
   ever MASK the credential prefix (`redact.rs:50`, `:111`, `:134`; the `@`
   boundary at `redact.rs:56` and the userinfo split at `redact.rs:138` are
   the same masking sites); the host and
   path are not secrets and stay legible. `redact_url` uses `url`'s
   `set_username`/`set_password` whenever parsing succeeds, so an IPv6 host
   stays bracketed and every component is handled per RFC 3986; the fallback
   is reached only when that fails. Test vectors: `redact.rs`'s own tests for
   both paths, including the behaviourally-guarded
   `masking_fallbacks_do_not_reconstruct_a_secret` (a host/substring present
   in the low-entropy input must never be reconstructed from the raw text).
   The lexical guard line-pins all five sites by name: the three
   `find("://")` scans (`redact.rs:50`, `:111`, `:134`) plus `rfind('@')`
   (`redact.rs:56`) and `rsplit_once('@')` (`redact.rs:138`).
6. **SRT query split** (`multimux/src/push/srt.rs`), which does NOT use
   `url`'s `query_pairs()` for the query portion of an opaque `srt://` URL.
   Verified against the `url` crate (2.5.8): a Haivision
   `streamid=#!::r=stream,m=publish` value contains an unencoded `#`, and
   `url` treats everything from that `#` on as the fragment — `query_pairs()`
   yields `("streamid", "")` and the real `streamid` moves to the fragment,
   cutting the value short. `srt-runtime`'s `as_stream_id` reads the ID from
   the handshake extension block and is opaque (it never parses a URL), so no
   crate in the workspace covers this form; `percent-encoding` alone decodes
   but does not split. The split is therefore manual (`strip_prefix("srt://")`
   + `split_once('?')`, `srt.rs:81-82`), while the AUTHORITY is still parsed by
   `url::Url::parse` (`srt.rs:146`) so IPv6 bracketing is the parser's job.
   Test vectors: `srt.rs`'s `parse_srt_url_for_test` cases, including the
   unencoded-`#` Haivision form and the percent-encoded (`%23`) form. The
   lexical guard line-pins the `strip_prefix("srt://")` site by name.
7. Anything the plan finds infeasible is escalated to the owner, never
   silently kept.

## 10. Risks

- **broadcast-auth Digest via `http-auth`.** Unverified (§4 SP2.4); it is the
  plan's first task, with the fallback defined.
- **multimux W2 is large.** It may be split into W2a (HTTP/WebRTC: SP2, SP4,
  defects 1–3) and W2b (ingest/pull/push: SP1, SP6) if the first reviewer pass
  shows the diff is unreviewable as one unit.
- **Trunk lock split.** It happens only if measured; it may land as "measured,
  no change".
- **sdp-types `Session::write` byte layout** may differ from the hand-built
  SDP (e.g. line order). Semantic equality is acceptable only with each
  difference listed and the interop suites green.
