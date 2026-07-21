# multimux 0.3.0 — 2026-07-21

**The hub.** `multimux` grows from a single-input (RTSP-pull), single-output
(LL-HLS-only) origin into a multi-input × multi-output just-in-time
repackaging HTTP origin, with shared server-side auth, production
observability, and an external plugin registry — closing out the
multimux-hub epic (issue #663).

## Highlights

### Multi-input

A route's ingest transport is now a tagged `config::InputSpec` (breaking
config change — see Migration): **RTSP** pull (unchanged), raw **RTP/UDP**
(uni/multicast, out-of-band SDP), **MPEG-TS/UDP** (uni/multicast, in-band
PMT), **MPEG-TS/HTTP** (streaming GET), and **HLS-pull** (wraps
`ll-hls-runtime`'s client engine). `TsHttp`/`HlsPull`/`Rtsp` each accept an
optional client-side `auth` (`AuthSpec`: username/password, answered as
Basic or Digest per the server's challenge, or a bearer token) via the
shared `broadcast-auth` crate.

### Multi-output, ingest-once

A route's `outputs: Vec<OutputKind>` (`"llhls"`/`"dash"`/`"ll_dash"`,
per-route, defaulting to LL-HLS only) selects which delivery protocol(s) to
serve the *same* ingested/segmented media as — no per-output re-mux. **DASH**
renders a live `$Number$`-addressed MPD via `transmux::DashPackager`.
**LL-DASH** (`manifest-ll.mpd`) is a discrete-parts-signalled low-latency MPD
addressing the existing LL-HLS-shaped parts (true chunked-transfer CMAF
LL-DASH remains future work).

### Shared output auth

One `Config::output_auth` (`OutputAuthSpec`: Basic/Digest/Bearer, or
**`Forwarded`** for a trusted reverse proxy forwarding an already-
authenticated username) now gates **every** media output route across
**every** configured route — e.g. 40 cameras under `/camN/index.m3u8`, one
shared credential. Independent of, and unrelated to, each route's own
ingest auth. Ops endpoints (`/healthz`/`/readyz`/`/metrics`) are never
gated.

### External scheme plugin registry

A third-party crate can add a new input, output, or output-auth scheme
**without editing multimux**, wired purely via config JSON: new
`Custom { type_tag, params }` variants on `InputSpec`/`OutputKind`/
`OutputAuthSpec`, resolved at `serve_with_registry(config, registry)` time
via a `registry::SchemeRegistry` the embedding application builds. See
`examples/custom_scheme.rs`.

### Production hardening

- **Supervised route lifecycle** — capped exponential-backoff reconnect on
  connect failure, pipeline error, or source EOF (was: die on first
  failure, freeze the served playlist forever).
- **HTTP resource limits + ingest timeouts** (issue #663 P5, audit-
  concurrency #3 / audit-ingest #3): per-request timeout, concurrency
  bound, and request-body cap on the HTTP listener; connect/read timeouts
  on every ingest source. All configurable, all default to the pre-#663
  behaviour.
- **Prometheus metrics** (`GET /metrics`) + `GET /healthz`/`GET /readyz`.
- **Structured errors, secret redaction, and `tracing`** throughout —
  `multimux-cli` owns subscriber init; the library only emits events.
- **Configurable `playlist_name`** — serve the LL-HLS media playlist under
  any `*.m3u8` name (default unchanged: `media.m3u8`).
- **LL-HLS spec-conformance fixes** (RFC 8216bis): `#EXT-X-TARGETDURATION`
  now reflects the real max segment duration, not just the configured
  target; blocking-reload `_HLS_msn` semantics corrected; abusive
  `_HLS_msn`/`_HLS_part` combinations rejected with `400` instead of always
  blocking; `Cache-Control` + permissive CORS on every response.
- **Fixed a self-deadlock**: `GET /media.m3u8` locked `MediaStore`'s mutex
  reentrantly on every request (found by the first real HTTP round-trip
  test against the endpoint).

### Architecture

The LL-HLS origin engine (rolling window, blocking-reload/part-availability
decision logic, playlist rendering) moved to the new `ll-hls-runtime` crate;
`multimux` is now a thin tokio+axum adapter over it, mirroring how it
already wraps `rtsp-runtime` on the input side. Behaviour-preserving.

## Breaking changes

- **Config**: `Route::rtsp_url: String` → `Route::input: InputSpec` (JSON:
  `"input": { "type": "rtsp", "url": "..." }` instead of a bare `"rtsp_url"`
  key).
- `output::OutputKind` no longer derives `Copy`/`PartialEq`/`Eq`/`Hash`
  (its new `Custom` variant carries a `serde_json::Value`) — compare via
  `.name()` or `matches!` instead of `==`; `name()` now returns `&str`
  rather than `&'static str`.
- `OutputAuthSpec::to_credentials` replaced by `build_verifier(realm)`
  (`pub(crate)` — internal only).
- `InputSpec`/`AuthSpec`/`OutputAuthSpec`/`OutputKind` are now
  `#[non_exhaustive]`.
- `output::llhls::{master_playlist, media_playlist}` narrowed from `pub` to
  `pub(crate)`.
- `MultimuxError`'s stringly `Config(String)`/`Source(String)` variants are
  replaced by field-carrying variants (`ConfigRead`/`ConfigParse`/
  `ConfigInvalid`/`Connect`/`Protocol`/`Sdp`/`Auth`/`Depay`) so callers can
  match on failure *kind* instead of parsing a string.

## Migration

Update every route in a JSON config from:

```json
{ "name": "cam1", "rtsp_url": "rtsp://host/stream1" }
```

to:

```json
{ "name": "cam1", "input": { "type": "rtsp", "url": "rtsp://host/stream1" } }
```

`outputs`, `auth`, and every other new field default to the pre-#663
behaviour (LL-HLS only, no client-side auth, `media.m3u8`) when omitted.

## Compatibility

MSRV 1.86. New dependencies: `ll-hls-runtime` (path, `0.1`), `broadcast-auth`
(path, `0.1`), `tower`/`tower-http` (HTTP resource limits), `metrics`/
`metrics-exporter-prometheus` (observability), `tracing`.
