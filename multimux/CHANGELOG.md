# Changelog

## [Unreleased]

### Added
- **External scheme plugin registry** (issue #663): a third-party crate can
  now add a new input, output, or output-auth scheme to the multimux origin
  **without editing multimux**, wired purely via config JSON. Built-ins
  (RTSP/RTP/TS-UDP/TS-HTTP/HLS-pull inputs; LL-HLS/DASH/LL-DASH outputs;
  Basic/Digest/Bearer/Forwarded output-auth) are unchanged — the typed,
  validated fast path. Extension is additive:
  - New `Custom { type_tag, params }` variants on `config::InputSpec` (JSON
    `{ "type": "custom", "type_tag": "...", "params": { ... } }`),
    `output::OutputKind` (JSON `{ "custom": { "type_tag": "...", "params":
    { ... } } }` — not internally tagged like the other two, since the other
    `OutputKind` variants are plain strings), and `config::OutputAuthSpec`
    (JSON `{ "scheme": "custom", "type_tag": "...", "params": { ... } }`).
    `params` is an opaque `serde_json::Value`, always structurally valid at
    `Config::validate` time — the registered factory validates it, at
    route-build time. Every `Custom` variant's hand-written `Debug` shows
    `type_tag` but always redacts `params` as `"<params>"` (it may carry an
    external scheme's credentials).
  - A new `registry::SchemeRegistry` — built by the embedding application,
    never by multimux itself — mapping each `type_tag` to a factory closure
    that builds the real thing from the opaque `params`: `register_input`/
    `register_output`/`register_auth` (and their `input`/`output`/`auth`
    lookups). `InputFactory` closures construct their own concrete
    `SourceConnector` and spawn `supervise` themselves (returning its
    `JoinHandle`) rather than returning a connector — `SourceConnector` is
    not object-safe, so this is how a factory erases the connector type;
    `OutputFactory`/`AuthFactory` return `Arc<dyn output::Output>`/
    `broadcast_auth::Verifier` directly (both already concrete/object-safe).
  - `origin::serve_with_registry(config, registry)` — `origin::serve(config)`
    is now `serve_with_registry(config, SchemeRegistry::new())`. An
    unregistered `Custom` `type_tag` fails route setup with the new
    `MultimuxError::UnknownScheme { kind, tag }` (`kind` is `"input"`,
    `"output"`, or `"auth"`) rather than panicking or silently no-opping.
  - New re-exports at the crate root for external factory authors:
    `SchemeRegistry`, `InputCtx`/`OutputCtx`/`AuthCtx`,
    `InputFactory`/`OutputFactory`/`AuthFactory`, `serve`/
    `serve_with_registry`, `supervise`/`SourceConnector`/`Backoff`,
    `Source`, `MediaStore`, `Output`, and the `broadcast_auth` crate itself
    (so a registered `AuthFactory` can build a `Verifier` without an
    external crate needing its own direct dependency on `broadcast-auth`).
  - New example `examples/custom_scheme.rs`: registers a custom input
    scheme with zero multimux edits.

### Changed (breaking)
- **`output::OutputKind` no longer derives `Copy`/`PartialEq`/`Eq`/`Hash`**
  (only `Debug`/`Clone`/`Deserialize`/`Serialize` remain): its new `Custom`
  variant carries a `serde_json::Value`, which is `Clone` but not `Copy`.
  Compare `OutputKind` values via `.name()` or `matches!` instead of `==`.
  `OutputKind::name()`'s return type changed from `&'static str` to `&str`
  (`Custom` labels itself by its own `type_tag`, borrowed from `self`).

### Added
- **`OutputAuthSpec::Forwarded` — reverse-proxy forwarded-auth output-auth
  scheme** (issue #663 extensibility wave part 1, built on
  `broadcast_auth::Verifier::forwarded`): configures the shared output-auth
  gate to trust a fronting reverse proxy that has already authenticated the
  caller, rather than checking a Basic/Digest/Bearer credential itself.
  JSON: `{ "scheme": "forwarded", "user_header": "X-Forwarded-User",
  "forwarded_for_header": "X-Forwarded-For" }` — both fields optional,
  defaulting to `X-Forwarded-User`/`Some("X-Forwarded-For")`; set
  `forwarded_for_header: null` to disable reading it. A request is allowed
  iff `user_header` is present and non-empty; `forwarded_for_header`, if
  set, is read for tracing/observability only — no trust decision is made
  from it. **Safe ONLY behind a trusted reverse proxy that strips any
  client-supplied copies of both headers before forwarding** — see
  `OutputAuthSpec::Forwarded`'s doc comment. `output_auth_gate` now builds a
  `broadcast_auth::RequestContext` carrying every request header (not just
  `Authorization`) plus the transport peer address (via
  `into_make_service_with_connect_info`, wired in `serve`), so any
  `Verifier` scheme — this one included — can see beyond `Authorization`.
- **`InputSpec`/`AuthSpec`/`OutputAuthSpec`/`output::OutputKind` are now
  `#[non_exhaustive]`** (issue #663 extensibility wave part 1): a future
  ingest transport, client-auth scheme, output-auth scheme, or delivery
  protocol can be added later without it being a breaking change for
  external matches on these types.

### Changed (breaking)
- **`OutputAuthSpec::to_credentials` replaced by
  `OutputAuthSpec::build_verifier(realm)`** (`pub(crate)`, so only affects
  this crate's own `serve`): returns the configured
  `broadcast_auth::Verifier` directly rather than a `Credentials` value —
  needed because `Forwarded` has no `Credentials` mapping at all (no
  username/password/token, no challenge/response round-trip).

### Added
- **Shared output auth** (issue #663 "shared output auth",
  `docs/superpowers/specs/2026-07-18-multimux-hub-design.md`): one
  Basic/Digest/Bearer credential can now gate **every** media output route
  (`/{stream}/…` — manifests and init/segment/part bytes alike, across every
  configured route, e.g. 40 cameras under `/camN/index.m3u8`) via a new
  `Config::output_auth` (`config::OutputAuthSpec`, JSON tagged on `scheme`:
  `{ "scheme": "basic"|"digest"|"bearer", ... }`). Independent of, and
  unrelated to, each route's own ingest auth (`config::AuthSpec`/URL
  userinfo) — one output credential guards the whole origin regardless of how
  differently each camera authenticates upstream. Built on the new
  `broadcast_auth::Verifier` (the server-side challenge+verify half,
  promoted out of `testutil`'s test-only mock — see that crate's changelog).
  Missing/wrong credentials get `401` + `WWW-Authenticate` (Basic/Digest
  challenge, or the bare `Bearer` token for Bearer); `output_auth: None` (the
  default) leaves every route open, unchanged from pre-#663 behaviour.
  **Ops endpoints (`/healthz`/`/readyz`/`/metrics`) are never gated** — load
  balancer probes and metrics scraping stay open regardless of
  `output_auth`. CORS/`Cache-Control` headers still apply to a `401`
  response from this gate (needed for a cross-origin browser client to see
  the status/challenge at all, not just a successful response).
- **Configurable `playlist_name`** (issue #663 "configurable
  `playlist_name`"): a new `Config::playlist_name` (default `"media.m3u8"`)
  names the LL-HLS media playlist filename served at
  `/{stream}/{playlist_name}`; `master.m3u8`'s `#EXT-X-STREAM-INF` reference
  follows suit (`output::llhls::LlHlsOutput::new`). Validated non-empty,
  `.m3u8`-suffixed, slash-free, and not `"master.m3u8"` (which would collide
  with the fixed master-playlist route). `master.m3u8`'s own name is not
  configurable; DASH's `manifest.mpd` is unaffected. Breaking (internal):
  `LlHlsOutput` is no longer a unit struct — use `LlHlsOutput::default()` (or
  `OutputKind::build()`) for the pre-existing `/media.m3u8` behaviour, or
  `LlHlsOutput::new(name)`/`OutputKind::build_with_playlist_name(name)` for a
  configured name; `output::llhls::{master_playlist, media_playlist}` are
  narrowed from `pub` to `pub(crate)` (their `State` type changed shape, and
  nothing outside this crate called them directly). Depends on
  `ll_hls_runtime::server::master_playlist_m3u8` now taking the media
  playlist's filename as an argument (see that crate's changelog).
- **RTSP config-auth (`with_auth`) Digest coverage against a real server**
  (the gap flagged in the client-auth story): a new loopback test drives
  `source::rtsp::RtspSource` with config-supplied (not URL-userinfo) Digest
  credentials against a mock server verified by the real
  `broadcast_auth::Verifier`, proving the `with_auth` -> `ClientSession`
  wiring end-to-end (success and wrong-password cases), mirroring
  `rtsp-runtime/tests/io_loopback.rs::digest_auth_over_loopback`.
- **Config-supplied + Bearer credentials, finishing client-side
  multi-scheme auth** (issue #663 — completes the P3c "Shared auth layer"
  story): `InputSpec::Rtsp`/`TsHttp`/`HlsPull` each gained an optional
  `auth` field (`config::AuthSpec` — either `{ username, password }` or
  `{ bearer_token }`), config-parseable via `--config <FILE>`. A Bearer
  token has no URL-userinfo form, so this is the *only* way to supply one;
  when both a config `auth` and URL userinfo are present, config wins
  (`source::http_auth::resolve_credentials`, now used by `RtspSource`,
  `TsHttpSource`, and `HlsPullSource` alike, each via a new
  `with_auth(Option<Credentials>)` builder mirroring `with_timeouts`).
  `AuthSpec`'s `Debug` redacts both `password` and `bearer_token`;
  `Config::validate` rejects an empty `username`/`bearer_token` (an empty
  `password` is accepted — some devices genuinely use one). Every
  pre-existing config still parses unchanged (`#[serde(default)]`).
  - **Digest/Basic/Bearer now proven end-to-end**, not just unit-tested in
    isolation: a new test-only mock auth server (`testutil`, gated
    `#[cfg(test)]`) gates a real axum router behind any of the three
    schemes — Digest verification is a real, independent RFC 7616 §3.4.1
    computation (not a literal-string match), so a client with the wrong
    password genuinely gets rejected. `source::ts_http` and
    `source::hls_pull` each gained Basic/Digest/Bearer/wrong-credentials
    tests driving the real `TsHttpSource`/`HlsPullSource` against it, plus a
    `config_auth_overrides_wrong_url_userinfo` precedence test.
  - No change needed to answer Digest's re-challenge-on-every-request
    concern: `ll_hls_runtime::client::tokio_client::TokioClient` already
    caches its Digest `Authenticator` across requests (from P3c), and
    `TsHttpSource`'s streaming GET only ever makes one request per
    `connect()`, so there was nothing further to cache there.
- **DASH output alongside LL-HLS, from the same shared CMAF segments**
  (issue #663 P4 — `docs/superpowers/specs/2026-07-18-multimux-hub-design.md`,
  "DASH output"): one ingested stream can now serve LL-HLS *and* MPEG-DASH
  simultaneously, both reading the exact same `MediaStore`-produced
  init/segment bytes (ingest-once, many-outputs — no per-output re-mux).
  - **The multi-output nest collision fix (the load-bearing refactor)**: two
    `Output`s each mounting their own `/:file` catch-all under the same
    `/{stream}` nest previously panicked axum. Fixed by splitting
    responsibilities: the `Output` trait's `router` method is now
    `manifest_routes` — each output contributes *only* its manifest
    route(s) (`master.m3u8`+`media.m3u8` for LL-HLS, `manifest.mpd` for
    DASH) — while the init/segment/part byte serving (`init-*.mp4`/
    `seg-*.m4s`/`part-*.m4s`, protocol-neutral since both outputs are
    fMP4/CMAF) moves to a new shared `origin::resource` route, mounted
    **once per stream** by `origin::router` (merging every output's
    manifest routes with the one shared resource route, then `nest`ing the
    merged router once — instead of nesting per-output). LL-HLS's URLs and
    behaviour (routes, blocking reload, `Cache-Control`/CORS policy) are
    unchanged; the shared `Cache-Control`/CORS middleware (generalized to
    treat `.mpd` the same as `.m3u8`) now lives at the origin level so it
    covers the shared resource route too.
  - `output::dash::DashOutput`: renders a live (`type="dynamic"`) MPD via
    `transmux::dash::DashPackager`, `$Number$`-addressed `SegmentTemplate`
    (not `$Time$`/`SegmentTimeline` — the store's `seg-{track}-{seq}.m4s`
    filenames are sequence-numbered, not time-addressed, so `$Number$` is
    the only mode whose URIs the shared resource route actually resolves),
    with `minimumUpdatePeriod`/`timeShiftBufferDepth`/
    `availabilityStartTime` derived from the store's target duration/window/
    construction time. Single-rendition model matching LL-HLS's own
    `DEFAULT_TRACK_ID` convention: the `Representation`'s `@id` is forced to
    `DEFAULT_TRACK_ID` regardless of the source's own track numbering, so
    `$RepresentationID$` substitution produces the same `init-1.mp4`/
    `seg-1-<N>.m4s` filenames LL-HLS already references. **True chunked-CMAF
    LL-DASH (`transmux::LlDashPackager`/`LlSegmenter`) is not implemented**
    — the store's `part-*.m4s` files are LL-HLS-shaped, not CMAF byte-range
    chunks; P4.2 below ships a signalled-MPD LL-DASH output addressing those
    existing parts instead, with true chunked transfer tracked as P4.3.
  - `ll_hls_runtime::server::MediaStore` gained the accessors a DASH
    renderer needs beyond LL-HLS's own bytes+timing: `set_track_specs`/
    `track_specs` (recorded once by `pipeline::run_pipeline` so DASH can
    build a real RFC 6381 `codecs` string), `window_segments` (a
    protocol-neutral snapshot of the closed-segment window), `created_at`
    (the live presentation's `availabilityStartTime` anchor); the previously
    crate-private `target_duration_secs`/`part_target_ms` accessors are now
    `pub` for the same cross-`Output` reason.
  - Config: `config::Route::outputs: Vec<output::OutputKind>` selects which
    protocol(s) to serve a route as (`"llhls"`/`"dash"`, per-route rather
    than a single global default — different routes may reasonably want
    different output sets), defaulting to LL-HLS only so every existing
    config is unaffected. `Config::validate` rejects an empty `outputs`
    list. `multimux-cli` gained `--outputs llhls,dash` (and the `--dash`
    shorthand for "both") on the single-route quick start.
- **LL-DASH output (low-latency DASH signalling)** (issue #663 P4.2 —
  `docs/superpowers/specs/2026-07-18-multimux-hub-design.md`, "LL-DASH"): a
  new `output::ll_dash::LlDashOutput`/`OutputKind::LlDash` (`"ll_dash"`)
  renders `manifest-ll.mpd`, an LL-DASH-**signalled** MPD carrying
  `availabilityTimeOffset`, `<ServiceDescription><Latency target="…"/></ServiceDescription>`
  (ISO/IEC 23009-1 §5.13.2), and a `minimumUpdatePeriod` tuned to the part
  target — served at its own path (not a mode flag on `manifest.mpd`) so a
  route can enable `dash` (DVR) and `ll_dash` (live edge) together.
  - **Scope decision: discrete-parts signalling, not true chunked-transfer
    LL-DASH.** As flagged by P4's own follow-up note, the store's
    `part-*.m4s` files are LL-HLS-shaped (a whole extra fMP4 `moof`+`mdat`
    per part), not CMAF byte-range chunks within one in-progress segment —
    wiring `transmux::LlDashPackager`/`LlSegmenter` for *true* chunked
    delivery needs a second, chunk-shaped segmenter output, a larger lift
    than this story's scope. Instead, `LlDashOutput` re-addresses the
    **existing** parts: its `SegmentTemplate` uses `$Number$` addressing
    with `@duration` = the real part target (not the whole-segment target),
    `startNumber="0"`, and a media template that bakes the in-progress
    segment's sequence number in as literal text (refreshed on every
    fetch — the MPD is always `type="dynamic"`, never cached) around the
    real `$RepresentationID$`/`$Number$` tokens, so a real client's
    substitution produces exactly the `part-{track}-{seq}.{idx}.m4s`
    filenames the shared resource route already serves for `ll_hls`. This
    covers **only the live edge** (no `timeShiftBufferDepth` — an absent
    value is spec-honest "unknown", not a fabricated DVR window this
    origin cannot serve); pair with `dash`'s `manifest.mpd` for seek-back.
    `availabilityTimeOffset` is honestly `"0"`: a part is produced
    atomically (never partially available), so the low-latency win here
    comes from the small nominal segment(=part) duration, not partial
    delivery — reusing `transmux::LlDashPackager`'s `segment − chunk`
    formula would misrepresent that, so this module hand-rolls its own
    small `<ServiceDescription>`/`availabilityTimeOffset` XML injection
    instead. True chunked-transfer CMAF remains tracked as **P4.3**.
  - `ll_hls_runtime::server::MediaStore::latest_progress` (the in-progress
    segment's sequence number + live part count) is now `pub` (was
    `pub(crate)`) for the same cross-`Output` reason as `window_segments`/
    `track_specs` before it.
  - Config: `outputs: ["llhls", "dash", "ll_dash"]` is now accepted;
    `Config::validate`/serde behave the same as any other `OutputKind`
    (unknown tokens rejected, empty `outputs` rejected).
- **Generalized input model + UDP-family ingest** (issue #663 P3a —
  `docs/superpowers/specs/2026-07-18-multimux-hub-design.md`, "Input
  adapters"): a route's ingest transport is now a tagged `config::InputSpec`
  (`Rtsp { url }` / `Rtp { addr, sdp, multicast_group }` / `TsUdp { addr,
  multicast_group }`, `#[serde(tag = "type", rename_all = "snake_case")]`),
  replacing the RTSP-only `Route::rtsp_url` field — a **breaking config
  change**: JSON routes now nest under `"input": { "type": "rtsp", "url":
  ... }` instead of a bare `"rtsp_url"` key. `origin::serve` dispatches each
  route to the matching `SourceConnector` with one `match` arm per
  `InputSpec` variant (kept monomorphized, not boxed, since each connector's
  `Source` associated type differs) — reconnect/backoff/health via
  `origin::supervisor::supervise` applies identically to every input kind.
  - `source::rtp_udp::RtpUdpSource` — raw RTP over UDP (uni/multicast, no
    RTSP control plane): binds a `tokio::net::UdpSocket` (+ optional
    multicast join via the new `source::udp::bind_udp` helper shared with
    `TsUdpSource`), parses the configured out-of-band SDP with the *same*
    `source::sdp::parse_sdp_tracks` RTSP already uses (no parallel SDP
    implementation), and depayloads with `transmux::RtpStreamDepacketiser`
    exactly as `source::rtsp::RtspSession` does. Since raw RTP/UDP has no
    RTSP interleaved-channel framing, incoming packets are routed to their
    track by RTP payload type (RFC 3550 §5.1) matched against each SDP
    media's declared payload type — `source::TrackInit` gained a
    `payload_type` field (populated identically for both the RTSP and raw-RTP
    ingest paths, since both share `parse_sdp_tracks`) and
    `source::sdp::load_sdp` loads an SDP body from either inline text or an
    `@path` file reference (re-read on every reconnect).
  - `source::ts_udp::TsUdpSource` — MPEG-2 Transport Stream over UDP
    (uni/multicast): binds the same shared UDP transport, then feeds
    datagrams to `transmux::StreamingTsDemux` (the same streaming demux core
    every other TS consumer in this workspace drives) until the in-band PMT
    resolves every declared track (bounded by a 10 s connect timeout) — the
    TS-over-UDP analogue of RTSP's DESCRIBE step — before the pipeline builds
    its segmenter.
  - No new codec/container parsing in multimux: both sources are transport
    (socket bind + multicast join) plus wiring over transmux's existing
    `RtpStreamDepacketiser`/`StreamingTsDemux`.
  - `Config::validate` now validates every route's `InputSpec` fields (RTSP
    scheme, UDP address parseability, multicast-group IP validity, RTP SDP
    non-empty/parseable) in addition to the existing duplicate-name/timing
    checks.
- **HTTP-based ingest: TS-over-HTTP + HLS-pull** (issue #663 P3c / #717 —
  `docs/superpowers/specs/2026-07-18-multimux-hub-design.md`, "Input
  adapters" / "Shared auth layer"): two new `InputSpec` variants,
  `TsHttp { url }` and `HlsPull { url }`, both `http(s)://` URLs that may
  carry `user:pass@` userinfo (redacted in `Debug`, validated for scheme by
  `Config::validate`).
  - `source::ts_http::TsHttpSource` — MPEG-2 Transport Stream over a
    streaming HTTP GET (chunked/progressive `reqwest`, `stream` feature):
    reads response chunks into `transmux::StreamingTsDemux` until the
    in-band PMT resolves every declared track (mirrors `TsUdpSource`'s own
    connect-time PMT wait, bounded the same 10 s). Unlike UDP, the HTTP body
    stream *does* end — `next_samples` returns `Ok(None)` on end-of-stream,
    so `origin::supervisor::supervise` reconnects exactly as for any other
    source's EOF.
  - `source::hls_pull::HlsPullSource` — wraps
    `ll_hls_runtime::client::tokio_client::TokioClient` (the sans-IO LL-HLS
    playback client engine, driven over real HTTP) as a `SourceConnector`/
    `SampleSource`: `connect()` drives the client until its first
    `Output::Init`, recovering the pulled stream's `TrackSpec`s by feeding
    those init bytes through `transmux::Fmp4Demux` once (the *same* demuxer
    the client itself already uses internally — no hand-rolled `moov`
    parse); `next_samples()` relays `Output::Samples` straight through. No
    re-demuxing: the client's own `Fmp4Demux`-based decode is reused
    verbatim.
  - `source::http_auth` — shared auth glue for both HTTP sources: reqwest
    answers Basic/Bearer natively, but not Digest (RFC 7616), so
    `authenticated_get` sends once and, on a `401`, answers the
    `WWW-Authenticate` challenge via the new `broadcast-auth` crate (issue
    #663 P3b) before resending — the same shared `Credentials`/
    `Authenticator` model `rtsp-runtime` already uses. Credentials come from
    the ingest URL's userinfo (mirrors `source::rtsp`'s own handling,
    generalized to any URL).
  - `ll-hls-runtime`'s `client::tokio_client::TokioClient` was itself
    upgraded in lockstep to authenticate via `broadcast-auth` (Basic/Digest/
    Bearer, replacing its previous ad hoc Basic/Bearer-only `Auth` enum) —
    see `ll-hls-runtime`'s own changelog — so `HlsPullSource` gets Digest
    support for free rather than multimux re-implementing the challenge/
    response for the pull path.
  - No new codec/container parsing in multimux: `ts_http` is transport
    (streaming GET) plus `StreamingTsDemux`; `hls_pull` is a thin wrapper
    over `ll-hls-runtime`'s existing client engine + `Fmp4Demux`.

### Security
- **UDP ingest read-timeout** (issue #663 P5.2, audit-ingest #3):
  `source::rtp_udp::RtpUdpSource`/`source::ts_udp::TsUdpSource`'s
  `next_samples()` previously called `UdpSocket::recv` with no timeout — a
  source that stopped sending (dropped multicast feed, wedged encoder) left
  the read pending forever, so `origin::supervisor::supervise` never saw an
  error to reconnect on. Both sessions' per-datagram `recv` is now wrapped in
  `tokio::time::timeout(self.read_timeout, …)`; on expiry `next_samples()`
  returns the same recoverable `MultimuxError::Connect` the supervisor
  already reconnects on for every other read error. `RtpUdpSource` gained
  the `timeouts: IngestTimeouts` field + `with_timeouts` builder it was
  previously missing (mirroring `TsUdpSource`/`RtspSource`); no config or
  behaviour change for a healthy source (default read timeout unchanged at
  30 s).
  - **Deferred (documented, not implemented this pass)**: RTCP Sender
    Report -> wall-clock A/V sync (issue #663 P5.2, audit-ingest #9/#10) —
    `source::rtsp::route_channel` (the interleaved RTCP channel) and
    `source::rtp_udp::RtpUdpSource::connect` (the RTCP companion UDP port)
    each still discard/never bind RTCP; both carry a
    `// TODO(P5.3): RTCP SR wallclock A/V sync` at the exact drop point.
    Judged too large a lift to land safely alongside the bounded-buffer and
    read-timeout hardening above (it would mean redesigning
    `transmux::rtp_stream`'s per-track timing/rebase model, not just this
    crate) — raw per-track RTP-timestamp rebasing is unchanged.

### Changed
- **LL-HLS origin engine moved to `ll-hls-runtime::server`** (issue
  #663/#717 Stage 2 —
  `docs/superpowers/specs/2026-07-18-multimux-hub-design.md`, "ll-hls-runtime
  — client + server in one crate"): `multimux` is now a thin tokio+axum
  adapter over the sans-IO engine, mirroring how it already wraps
  `rtsp-runtime` on the input side. Behaviour-preserving — every existing
  test still passes, same served bytes/URLs/timing:
  - `store::MediaStore`/`store::HealthState` are now re-exports of
    `ll_hls_runtime::server::{MediaStore, HealthState}` (the rolling window,
    the part-404-boundary fix, and health tracking moved there verbatim);
    `crate::store::...` call sites are unaffected.
  - `output::llhls` no longer renders playlists or decides blocking-reload/
    part-availability outcomes itself — it calls
    `MediaStore::resolve_playlist`/`resolve_resource` and drives the actual
    bounded `.await` (the one thing the sans-IO engine can't do): on
    `WouldBlock`, it registers `MediaStore::listen()` before re-resolving (no
    missed-wakeup race), then awaits the listener under its own
    `tokio::time::timeout` (still the 5 s `BLOCKING_RELOAD_TIMEOUT`). The
    reentrant-lock deadlock fix in playlist rendering is preserved (now in
    `ll_hls_runtime::server::media_playlist_m3u8`).
  - New dependency: `ll-hls-runtime` (path + version, `std` feature).

### Added
- **Supervised route lifecycle** (issue #663, P0.2+P0.3+P0.4): each route's
  ingest task is now driven by `origin::supervisor::supervise`, which
  reconnects with capped exponential backoff (`origin::supervisor::Backoff`,
  default 500ms min / 30s max / factor 2.0) on connect failure, pipeline
  error, *or* source end-of-stream, instead of the old one-shot task that
  died for good on the first failure (leaving the HTTP origin serving a
  frozen last playlist as `200 OK` forever). The connect step is abstracted
  behind `origin::supervisor::SourceConnector` (implemented for
  `source::rtsp::RtspSource`) so reconnect is testable without a real RTSP
  server. A route never gives up permanently by default — sources like
  cameras come back.
- **Store health** (`store::MediaStore::{health, set_health}` /
  `store::HealthState`): each route's `MediaStore` now tracks
  `Connecting`/`Live`/`Reconnecting`/`Failed`, set by the supervisor at each
  connect/pipeline transition; a state change bumps the store's existing
  progress watch so a blocked reader (e.g. an LL-HLS long-poll reload) wakes
  on a health transition too, not just new media.
- **Graceful shutdown**: `origin::serve` now installs a shutdown signal
  (Ctrl-C, plus `SIGTERM` on unix) that both drains in-flight HTTP requests
  via `axum::serve(..).with_graceful_shutdown(..)` and breaks every route's
  supervise loop; `serve` joins each supervisor task (aborting one that
  doesn't return within a short grace period) before returning `Ok(())`,
  rather than leaving ingest tasks running detached past shutdown.
- **Structured errors + secret redaction + tracing** (issue #663, P1a):
  - `error::MultimuxError` replaces the stringly `Config(String)`/
    `Source(String)` variants with field-carrying `thiserror` variants
    (`ConfigRead`/`ConfigParse`/`ConfigInvalid`/`Connect`/`Protocol`/`Sdp`/
    `Auth`/`Depay`, plus the existing `Transmux`/`Io`), so callers can match
    on failure *kind* instead of parsing a string, following the
    `rtsp-runtime` convention.
  - **Secret redaction**: an RTSP source URL's `user:pass@` userinfo can no
    longer leak into an error message, `Debug` output, or a log line.
    `config::Route` and `source::rtsp::RtspSource` now have manual `Debug`
    impls that redact `rtsp_url`/`url` to `***@host`; every connect-time
    error path (bad URL parse, connect/TLS/SNI failure, userinfo-stripping
    failure) redacts or uses the already userinfo-stripped URL rather than
    the raw credentialed one.
  - `tracing` throughout: `origin::supervisor::supervise` is
    `#[tracing::instrument]`ed per route (connect/live `info!`, disconnect/
    reconnect `warn!` with backoff delay + attempt count, health
    transitions logged) and `origin::serve` logs startup/shutdown/aborted
    supervisor tasks, replacing the ingest supervisor's `eprintln!`s. The
    library only emits events — `multimux-cli` owns subscriber init
    (`tracing-subscriber` `fmt` + `EnvFilter`, `RUST_LOG`-overridable,
    default `info`); the CLI's own top-level fatal-error report stays a
    plain `eprintln!` so it's never swallowed by a log filter.
- **Prometheus metrics + health/readiness endpoints** (issue #663, P1c):
  - New `prometheus` module: installs a single process-wide
    `metrics-exporter-prometheus` recorder (idempotent — safe to call from
    every `AppState::new`, including many tests sharing one process) and
    renders its snapshot for `GET /metrics`.
  - Metrics recorded throughout the crate via the `metrics` facade:
    `multimux_route_up` (gauge, `route`; mirrors `HealthState` — 1.0 while
    `Live`), `multimux_source_reconnects_total` (counter, `route`; bumped by
    `origin::supervisor::supervise` on each `Reconnecting` transition),
    `multimux_segments_produced_total`/`multimux_parts_produced_total`
    (counters, `route`; bumped in `pipeline::run_pipeline`, which now takes a
    `route: &str` parameter for this label),
    `multimux_active_blocking_requests` (gauge; inc/dec around
    `output::llhls`'s blocking `wait_for_progress`/`wait_for_part` waits via
    an RAII guard), and `multimux_http_requests_total`/
    `multimux_http_request_duration_seconds`/`multimux_bytes_served_total`
    (labels `route`, `path`, and `status` for the requests counter; recorded
    by a new `origin::router` global middleware layer for every request,
    root endpoints included). Cardinality is bounded on purpose: `route` is a
    configured stream name or `"unknown"`, `path` is one of a small fixed set
    of kinds (`playlist`/`segment`/`part`/`init`/`metrics`/`health`/`other`).
  - `GET /healthz` (liveness, always 200) and `GET /readyz` (readiness: 200
    once at least one configured route is `Live`, 503 otherwise) mounted at
    the origin root alongside `/metrics`, above the per-stream `/{stream}/`
    nests.
  - `origin::AppState` gained a `metrics_handle` field and an `AppState::new`
    constructor (replacing the old bare struct literal at every call site).

### Fixed
- **LL-HLS spec-conformance** (issue #663, P2 — RFC 8216bis):
  - `#EXT-X-TARGETDURATION` is now `round(max(configured target, max actual
    segment duration ever seen))`, not `ceil(configured target)`. The
    segmenter cuts on the next keyframe *after* the configured target, so a
    real segment routinely runs longer than the configured value — advertising
    the configured target alone under-declared TARGETDURATION and violated RFC
    8216bis §4.4.3.1 (a MUST: every Media Segment's rounded EXTINF ≤
    TARGETDURATION). `store::MediaStore` now tracks a lifetime
    `max_segment_duration` (never reset on window eviction) that the LL-HLS
    renderer folds into the tag.
  - Blocking-reload `_HLS_msn` semantics (§6.2.5.2): a bare `_HLS_msn` (no
    `_HLS_part`) now waits until segment `msn` is a fully-present **closed**
    Media Segment, rather than resolving as soon as the segment merely *opens*
    with one live part (the old `unwrap_or(0)` conflated it with
    `_HLS_part=0`). `_HLS_msn`+`_HLS_part` keeps the existing part-count
    semantics.
  - `_HLS_msn`/`_HLS_part` abuse bounds (§6.2.5.2): `_HLS_part` without
    `_HLS_msn`, or an `_HLS_msn` more than a small bound beyond the current
    live edge, is now rejected promptly with `400 Bad Request` instead of
    always blocking to the 5 s timeout and returning `200`.
  - `Cache-Control` + permissive CORS on every origin response: immutable
    `max-age=31536000, immutable` on init/segment/part byte ranges, `no-cache`
    on playlists, and `Access-Control-Allow-Origin: *` (+ methods/headers, with
    an `OPTIONS` preflight handler) on everything — browser LL-HLS players
    (hls.js) are commonly on a different origin than the API.
  - **`GET /media.m3u8` deadlocked on every request** (found by
    `ll-hls-client`'s issue #717 slice 5 acceptance test, the first thing in
    the workspace to ever drive this endpoint over a real HTTP round trip
    rather than calling `output::llhls::media_playlist_m3u8` directly):
    `media_playlist_m3u8` called `store::MediaStore::with_segments_and_parts`
    (which locks `MediaStore`'s internal `std::sync::Mutex`) and, from
    *inside* that closure, called `store.max_segment_duration()` — which
    locks the same, non-reentrant mutex again. Every request to `/media.m3u8`
    (blocking or not, empty store or not) self-deadlocked the handling task
    forever. `target_duration_secs()`/`max_segment_duration()` are now read
    *before* taking the `with_segments_and_parts` lock.

## [0.2.2] - 2026-07-18

### Fixed
- **LL-HLS preload-hint parts no longer 404 at every segment boundary.** The
  segmenter emits a segment's *final* part and closes the segment in the same
  step; `add_segment` then evicted that segment's parts from `live_parts`
  immediately — so the final part (exactly the one the `#EXT-X-PRELOAD-HINT`
  points at) existed for only microseconds, and the in-flight blocking part
  request that raced the close still 404'd. 0.2.1 made not-yet-produced parts
  *block*; this makes the just-produced final part *survive*: `add_segment` now
  moves a closed segment's parts into a bounded `recent_parts` buffer that
  `part_bytes` also checks, so the hinted final part is served (HTTP 200) after
  its segment closes instead of 404ing. Eliminates the per-segment 404 spam +
  the boundary latency bump. Bounded oldest-first like `live_parts`; closed
  parts are still not rendered in the playlist (the whole segment is).

### Fixed
- **LL-HLS preload-hint parts no longer 404.** A request for a Partial Segment
  the media playlist promised via `#EXT-X-PRELOAD-HINT` but that the origin had
  not produced yet returned `404` immediately, instead of holding the request
  open until the part became available (RFC 8216bis §6.2.2 / §6.3.1 blocking
  Partial-Segment delivery). Every low-latency client (hls.js, Safari native)
  therefore hammered the hinted part with 404s until it happened to exist,
  spamming errors and forcing a fall back to full-segment loads — defeating the
  low-latency path. The part byte handler now blocks (reusing the same progress
  watch as the blocking playlist reload) until the part is produced, or returns
  `404` *promptly* once its segment closes without it (a real segment boundary)
  or the blocking timeout elapses. Observed against a live on-camera stream.

### Breaking
- The bundled `multimux` **binary** (the RTSP→LL-HLS CLI) moved to a new
  dedicated crate, **`multimux-cli`**. `multimux` is now a **library only**
  (its `serve`/`config`/`origin`/`pipeline`/`source`/`store` API is unchanged).
  `cargo install multimux-cli` provides the `multimux` binary as before. The
  `cli` cargo feature (and the `clap` dependency) were removed from `multimux`.

## [0.1.0] - 2026-07-15
### Added
First release (issue #663): a live RTSP → LL-HLS just-in-time repackaging HTTP
origin — a thin client + server wrap around `rtsp-runtime` (RTSP pull) and
`transmux` (RTP depayload + LL-HLS CMAF segmentation).

- **Config** (`config::Config`/`Route`): CLI-first, with an optional JSON
  config file for multiple routes; `bind`, `target_duration_secs`,
  `part_target_ms`, `window_segments`, and `routes: [{ name, rtsp_url }]`;
  `Config::validate()` rejects empty route sets, duplicate stream names, and
  nonsensical timing/window values.
- **RTSP ingest** (`source::rtsp::RtspSource`/`RtspSession`): DESCRIBE → SETUP
  (interleaved TCP, one media per SETUP) → PLAY over
  `rtsp_runtime::io::AsyncRtspClient`; SDP → per-track `CodecConfig` via
  `transmux`'s SDP-fmtp helpers; interleaved RTP routed per channel into
  `transmux::RtpStreamDepacketiser`, yielding timed `Sample`s.
- **Per-route pipeline** (`pipeline::run_pipeline`): drives a `SampleSource`
  (real `RtspSession` or, for tests/examples, `MockSource`) through a
  `transmux::ll_hls::LlHlsSegmenter`, publishing every init segment, ready
  part, and ready segment into a `StreamStore`; flushes the buffered tail at
  end-of-stream.
- **`StreamStore`** (`store::StreamStore`): per-stream in-RAM rolling window
  (init segment + a bounded `VecDeque` of closed segments + the in-progress
  segment's live parts), oldest segment evicted on roll; a
  `tokio::sync::watch` bumped on every new part/segment drives blocking
  playlist reload; renders the LL-HLS media playlist per RFC 8216bis
  (`#EXT-X-PART-INF`/`#EXT-X-SERVER-CONTROL`/`#EXT-X-PART`/
  `#EXT-X-PRELOAD-HINT`), never advertising an `#EXTINF`/URI for an
  unclosed segment (§4.4.4.9).
- **HTTP origin** (`origin::{router, serve}`, axum): `master.m3u8`,
  `media.m3u8` (blocking reload on `_HLS_msn`/`_HLS_part`, RFC 8216bis
  §6.2.5.2, bounded so a stalled source can't hang a request forever), and a
  catch-all serving the dynamic `init-*.mp4`/`seg-*.m4s`/`part-*.m4s`
  filenames the playlist emits. `origin::serve(config)` wires one
  `StreamStore` + one spawned RTSP pipeline task per configured route, then
  binds and serves — a single bad/unreachable source logs to stderr and ends
  only that route's task, never the server.
- **CLI binary** (`multimux`, `cli` feature, on by default): `--config <FILE>`
  or the single-route quick start `--rtsp <URL> --name <NAME>`, plus
  `--bind`/`--target-duration`/`--part-ms`/`--window`, per
  `docs/CLI-STANDARD.md`.
- **Examples**: `serve_mock` (synthetic stream, no RTSP/network needed) and
  `serve_rtsp` (serves one real RTSP URL given on the command line).

### v1 scope
LL-HLS only (DASH/LL-DASH is v1.1); RTSP pull only (no SRT/TS/file ingest); no
per-viewer sessions/SSAI/manifest rewrites; no DVR/VOD/disk spill (RAM-only
rolling window); no TLS/auth (front it with a reverse proxy); no trick-play.
