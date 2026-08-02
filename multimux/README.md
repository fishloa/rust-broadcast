# multimux — multi-input, multi-output just-in-time repackaging origin

**multimux is a hub, not a single pipe.** It pulls/receives live media from
any of several ingest transports, and serves each ingested stream as any
combination of low-latency delivery protocols, from one in-process
tokio + axum HTTP origin. Muxing only: samples are opaque and are never
transcoded. Every route (one ingest → its served outputs) is independent —
a single instance can serve dozens of unrelated cameras/feeds side by side.

```text
  RTSP  ─┐                                          ┌─▶  LL-HLS  (media.m3u8 + parts)
  RTP   ─┤                                          │
  TS/UDP─┼─▶  ingest  ─▶  transmux (depay/segment) ──┼─▶  DASH     (manifest.mpd)
  TS/HTTP┤                     one route =                │
  HLS-pull┘               one ingest, N outputs           └─▶  LL-DASH  (manifest-ll.mpd)
```

## Inputs

Each route names one ingest transport (`InputSpec`):

| `type` | Transport | Notes |
| --- | --- | --- |
| `rtsp` | RTSP pull (DESCRIBE/SETUP/PLAY, interleaved TCP), via `rtsp-runtime` | optional `auth` |
| `rtp` | Raw RTP over UDP (uni/multicast) | needs an out-of-band SDP (inline or `@path`) for codec/fmtp |
| `ts_udp` | MPEG-2 TS over UDP (uni/multicast) | track set comes from the in-band PMT — no SDP needed |
| `ts_http` | MPEG-2 TS over a streaming HTTP GET (chunked/progressive) | optional `auth` |
| `hls_pull` | Pull a remote (LL-)HLS Media Playlist, via `hls-runtime`'s client | optional `auth` |
| `srt` | MPEG-2 TS over SRT, caller (dial out) or listener (bind + accept), via `srt-runtime` | track set from the in-band PMT; payload encryption out of scope |
| `dash_pull` | Pull a remote DASH MPD and its segments | optional `auth` |
| `smooth_pull` | Pull a remote Smooth Streaming manifest and its fragments | optional `auth` |
| `rtmp` | **Push** ingest: binds a listen port and accepts RTMP publishers (FLV), via `rtmp-runtime` | `app`/`stream_key` filters; concurrent publishers |

`rtsp` accepts `rtsps://` for RTSP over TLS. `rtmp` is the only *push* input —
nothing is dialled out to; it binds and accepts, and one stalled publisher does
not block another.

`rtsp`/`ts_http`/`hls_pull`/`dash_pull`/`smooth_pull` each accept an optional `auth` — either
`{ "username": "...", "password": "..." }` (answered as Basic or Digest,
whichever the upstream's own challenge asks for) or `{ "bearer_token":
"..." }` (RFC 6750; the only way to supply a bearer token, since it has no
URL-userinfo form). A username/password may instead ride the route's own
URL userinfo (`rtsp://user:pass@host/...`); an explicit `auth` always wins
over that.

Codecs: H.264 video + AAC audio (whatever `transmux`'s depayload/demux
supports — any missing codec/transport capability is a library gap fixed
upstream, in `transmux` or `rtsp-runtime`, never in this crate).

## Outputs

Each route selects which delivery protocol(s) to serve its ingested media
as (`outputs`, defaulting to `["llhls"]` — every pre-existing config is
unaffected), all reading the exact same segmented CMAF — ingest-once,
many-outputs, no per-output re-mux:

| `outputs` token | Served as | Manifest |
| --- | --- | --- |
| `"llhls"` | Low-Latency HLS (RFC 8216bis) | `master.m3u8` + `media.m3u8` (or the configured `playlist_name`) |
| `"dash"` | MPEG-DASH, `$Number$`-addressed | `manifest.mpd` |
| `"ll_dash"` | Low-latency DASH, true chunked-transfer CMAF (whole-segment `$Number$`, served over HTTP chunked transfer while in progress) | `manifest-ll.mpd` |

A route can enable more than one (e.g. `["llhls", "dash"]`), and different
routes may enable different sets.

## Served endpoints

One route ("stream") is served per configured `name`, under `/{stream}/...`:

| Endpoint | Description |
| --- | --- |
| `GET /{stream}/master.m3u8` | LL-HLS master playlist (if `llhls` is enabled). |
| `GET /{stream}/media.m3u8[?_HLS_msn=&_HLS_part=]` | LL-HLS media playlist (or the configured `playlist_name`). Blocking Playlist Reload (RFC 8216bis §6.2.5.2) when `_HLS_msn`/`_HLS_part` are present. |
| `GET /{stream}/manifest.mpd` | DASH manifest (if `dash` is enabled). |
| `GET /{stream}/manifest-ll.mpd` | Low-latency DASH manifest (if `ll_dash` is enabled). |
| `GET /{stream}/init-{track}.mp4` | fMP4 init segment (`moov`) — shared across every enabled output. |
| `GET /{stream}/seg-{track}-{seq}.m4s` | A full media segment: served whole (`Content-Length`) once closed, or streamed over **HTTP chunked transfer-encoding** while still in progress (issue #721 — `ll_dash`'s low-latency delivery). |
| `GET /{stream}/part-{track}-{seq}.{part}.m4s` | An LL-HLS partial segment of the in-progress segment (also how `ll_dash`'s chunked-transfer path internally sources the bytes it streams — never addressed directly by the LL-DASH MPD itself). |
| `GET /healthz` | Liveness — always `200`. Never gated by `output_auth`. |
| `GET /readyz` | Readiness — `200` once at least one route is live, `503` otherwise. Never gated by `output_auth`. |
| `GET /metrics` | Prometheus metrics. Never gated by `output_auth`. |

An unknown stream name, or a filename `multimux` doesn't recognize, returns
`404`.

## Shared output auth

One credential can gate **every** media output route (manifests and
init/segment/part bytes alike) across **every** configured route — e.g. 40
cameras under `/camN/media.m3u8`, one shared login — via `Config::output_auth`.
Independent of, and unrelated to, each route's own ingest `auth`. `None`
(the default) leaves every route open.

```json
{ "output_auth": { "scheme": "basic", "username": "ops", "password": "hunter2" } }
```

Schemes (`scheme` tag): `"basic"` / `"digest"` (username + password),
`"bearer"` (token), and `"forwarded"` — see below.

### Reverse-proxy deployment (`forwarded`)

When multimux sits behind a reverse proxy that already terminates TLS and
authenticates the caller (its own login, mTLS, an SSO gateway, ...), the
`forwarded` scheme trusts the proxy's own `X-Forwarded-User` (configurable)
header instead of checking a credential itself — no second login,
no `WWW-Authenticate` round-trip a direct client could answer:

```json
{
  "output_auth": {
    "scheme": "forwarded",
    "user_header": "X-Forwarded-User",
    "forwarded_for_header": "X-Forwarded-For"
  }
}
```

**Safe ONLY when the origin is reachable exclusively through a reverse
proxy that strips any client-supplied copies of `user_header` (and
`forwarded_for_header`, if set) before forwarding.** multimux performs no
such stripping and trusts every inbound header completely — if the origin
is *also* reachable directly, any client can set these headers itself and
bypass authentication entirely.

## Runtime admin API

Add/remove/list routes and reload the config file **without restarting the
origin** — restarting drops every live viewer on every route, not just the
one being changed. Opt-in: omit `admin` from the config (the default) and no
admin listener is ever bound, no admin route ever exists.

```json
{
  "bind": "0.0.0.0:8080",
  "admin": {
    "bind": "127.0.0.1:9090",
    "auth": { "scheme": "bearer", "token": "admin-secret" }
  },
  "routes": [ /* ... */ ]
}
```

### Security posture — read this before enabling it

- **Separate listener, always.** `admin.bind` **must differ** from `bind`
  (enforced by `Config::validate` — a config with the two equal is rejected
  at load time). The admin API is never reachable on the public media port.
  Bind it to `127.0.0.1` or a private management network, never `0.0.0.0`
  on a box with a public media port, unless a firewall in front of it
  already restricts access.
- **Auth is mandatory, not optional.** `admin.auth` is a plain
  `OutputAuthSpec` (same schemes as `output_auth` — Basic/Digest/Bearer/
  Forwarded/Custom), **not** `Option<OutputAuthSpec>`: a config that sets
  `admin.bind` without `admin.auth` fails to parse before the process ever
  binds a socket. There is no way to run an unauthenticated admin listener.
  Use a *different* credential from `output_auth` — the admin API can add
  and remove routes; media playback can only read them.

### Endpoints

| Method | Path | |
| --- | --- | --- |
| `GET` | `/admin/routes` | List every route + live status (`name`, input kind, outputs, health, `created_at`). |
| `GET` | `/admin/routes/{name}` | One route's status, or `404`. |
| `POST` | `/admin/routes` | Add a route. Body: the same `Route` JSON shape a config file's `routes[]` entries use. `409 Conflict` if `name` already exists (the existing route is left untouched); `400` if the body is malformed or fails validation (the route list is left exactly as it was). |
| `DELETE` | `/admin/routes/{name}` | Remove a route. `404` if unknown. New requests for `{name}` 404 immediately; any request already being served from it (an open LL-HLS long-poll, e.g.) completes normally against whatever had already landed — a graceful drain, not a dropped connection. Every *other* route keeps serving uninterrupted. |
| `POST` | `/admin/reload` | Re-read the config file this process was started with (`--config <FILE>` / `serve_config_file`) and converge: routes added, removed, and changed are applied; **a route whose config is byte-for-byte unchanged is never restarted.** Returns a summary: `{ "added": [...], "removed": [...], "changed": [...], "unchanged": [...] }`. |

Every mutation is validated *before* it is applied — a malformed or
unbuildable route never leaves the origin half-converged.

```bash
curl -u admin:hunter2 http://127.0.0.1:9090/admin/routes
curl -X POST -H 'Content-Type: application/json' \
  -d '{"name":"cam41","input":{"type":"rtsp","url":"rtsp://cam41.local/stream"}}' \
  http://127.0.0.1:9090/admin/routes
curl -X DELETE http://127.0.0.1:9090/admin/routes/cam41
curl -X POST http://127.0.0.1:9090/admin/reload
```

`POST /admin/reload` only works when the process knows its own config file
path (`multimux --config routes.json`, or `origin::serve_config_file`); an
origin started from an in-memory `Config` (`origin::serve`/
`serve_with_registry`, no file) rejects reload with a clear error — add/
remove/list still work normally.

## Config shape

```json
{
  "bind": "0.0.0.0:8080",
  "target_duration_secs": 4.0,
  "part_target_ms": 500,
  "window_segments": 8,
  "request_timeout_secs": 10.0,
  "max_concurrent_requests": 4096,
  "max_request_body_bytes": 16384,
  "ingest_connect_timeout_secs": 10.0,
  "ingest_read_timeout_secs": 30.0,
  "playlist_name": "media.m3u8",
  "output_auth": null,
  "admin": null,
  "routes": [
    {
      "name": "cam1",
      "input": { "type": "rtsp", "url": "rtsp://host/stream1" },
      "outputs": ["llhls"]
    }
  ]
}
```

Every field except `routes` has a default (`Config::default()`); every
route's `outputs` defaults to `["llhls"]`. `request_timeout_secs` must
exceed 5.0 (the LL-HLS blocking-reload cap) or a legitimate long-poll
request would be cut off by the HTTP layer before the LL-HLS engine gets a
chance to resolve it.

### 40-camera scenario

Many routes, one shared output credential, one process:

```json
{
  "bind": "0.0.0.0:8080",
  "output_auth": { "scheme": "digest", "username": "ops", "password": "hunter2" },
  "routes": [
    { "name": "cam1", "input": { "type": "rtsp", "url": "rtsp://cam1.local/stream" } },
    { "name": "cam2", "input": { "type": "rtsp", "url": "rtsp://cam2.local/stream" } }
    /* … cam3 … cam40 … */
  ]
}
```

Each camera is served at its own `/camN/media.m3u8`, independently
reconnected/supervised, all gated by the one `output_auth` credential.

## External scheme plugin registry

A third-party crate can add a new **input**, **output**, or **output-auth**
scheme without editing multimux at all, wired purely via config JSON:
`InputSpec::Custom { type_tag, params }`, `OutputKind::Custom { type_tag,
params }`, and `OutputAuthSpec::Custom { type_tag, params }`, resolved at
`serve_with_registry(config, registry)` time against a `registry::SchemeRegistry`
the embedding application builds (`register_input`/`register_output`/
`register_auth`). `origin::serve(config)` is `serve_with_registry(config,
SchemeRegistry::new())` — the empty registry, for the built-in schemes only.
See [`examples/custom_scheme.rs`](examples/custom_scheme.rs) for a
complete, runnable example.

## Quick start

Single-route quick start (one camera, no config file):

```bash
multimux --rtsp rtsp://cam.local/stream --name cam1
curl http://0.0.0.0:8080/cam1/master.m3u8
```

Multi-route JSON config file:

```bash
multimux --config routes.json
```

Every flag/field has a default (see `multimux --help` or
`multimux::config::Config::default()`); `--rtsp`/`--name` and `--config` are
mutually exclusive — pass one or the other.

## Production hardening

- **Supervised route lifecycle** — each route reconnects with capped
  exponential backoff on connect failure, pipeline error, or source EOF,
  rather than dying on the first failure.
- **HTTP resource limits + ingest timeouts** — per-request timeout,
  concurrency bound, and request-body cap on the listener
  (`request_timeout_secs`/`max_concurrent_requests`/
  `max_request_body_bytes`); connect/read timeouts on every ingest source
  (`ingest_connect_timeout_secs`/`ingest_read_timeout_secs`).
- **Structured errors + secret redaction + `tracing`** — no credential ever
  reaches a log line, error message, or `Debug` output.
- **Prometheus metrics + health/readiness** — see the served-endpoints
  table above.
- **Graceful shutdown** — Ctrl-C / `SIGTERM` drains in-flight requests and
  every route's ingest task before exiting.

## v1 limits (still out of scope)

- Per-viewer sessions, server-side ad insertion, manifest rewrites.
- DVR / VOD / disk spill (the window is RAM-only and rolls forward).
- Trick-play.

Additional documented limits inherited from the underlying streaming
depayloader (`transmux`'s `RtpStreamDepacketiser`, issue #700): low-delay
H.264 only (no B-frame reordering), one AAC access unit per RTP packet, and
packets must arrive in order.

See
[`docs/superpowers/specs/2026-07-18-multimux-hub-design.md`](../docs/superpowers/specs/2026-07-18-multimux-hub-design.md)
in the workspace root for the full hub design, and
[`docs/superpowers/specs/2026-07-14-multimux-design.md`](../docs/superpowers/specs/2026-07-14-multimux-design.md)
for the original v1 (RTSP→LL-HLS-only) design this hub replaced.

## Examples

```bash
# Serve one real RTSP source.
cargo run --example serve_rtsp -- rtsp://cam.local/stream

# Register a custom input scheme with zero multimux edits (drives a real
# synthetic Dialer/IngestSession through supervise_driver end to end).
cargo run --example custom_scheme
```

### Example configs

JSON files under [`examples/`](examples/) — each a realistic, valid
`multimux::config::Config` for `multimux-cli --config <file>` (deserialize +
`validate()` are guarded by `tests/example_configs.rs`, so they can't drift
from the config schema):

- [`webcam-fleet-40.json`](examples/webcam-fleet-40.json) — 40 routes
  (`cam1`..`cam40`) spanning all five ingest protocols (RTSP with per-camera
  Password/Bearer `auth`, RTP, TS/UDP multicast, TS/HTTP, HLS-pull), all
  served under one shared `output_auth` (Basic) — heterogeneous ingest, one
  uniform LL-HLS(+DASH) output surface.
- [`reverse-proxy.json`](examples/reverse-proxy.json) — `output_auth` using
  the `forwarded` scheme: TLS terminates at a fronting reverse proxy, and the
  origin trusts its `X-Forwarded-User` header instead of challenging clients
  itself (see [`OutputAuthSpec::Forwarded`](src/config.rs)'s trust-assumption
  docs before using this in production).
- [`multi-output.json`](examples/multi-output.json) — one RTSP ingest
  packaged to all three outputs (`llhls`, `dash`, `ll_dash`) from the same
  CMAF segments (issue #663 P4's "ingest-once, many-outputs").
- [`custom-scheme.json`](examples/custom-scheme.json) — an `InputSpec::Custom`
  route naming the `"demo"` scheme
  [`examples/custom_scheme.rs`](examples/custom_scheme.rs) registers.

## Spec

RFC 8216bis (HTTP Live Streaming, 2nd edition) — Low-Latency HLS:
`#EXT-X-PART` (§4.4.4.9), `#EXT-X-PART-INF`/`#EXT-X-SERVER-CONTROL`
(§4.4.3.7/§4.4.3.8), and Blocking Playlist Reload (§6.2.5.2). ISO/IEC
23009-1 (DASH) for the `dash`/`ll_dash` outputs. RTSP 1.0 (RFC 2326) for
RTSP ingest, via `rtsp-runtime`. RFC 7617/7616/6750 (Basic/Digest/Bearer)
for auth, via `broadcast-auth`.

## License

MIT OR Apache-2.0
