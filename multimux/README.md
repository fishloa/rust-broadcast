# multimux

[![crates.io](https://img.shields.io/crates/v/multimux.svg)](https://crates.io/crates/multimux)
[![docs.rs](https://img.shields.io/docsrs/multimux)](https://docs.rs/multimux)

Multi-input, multi-output just-in-time repackaging origin. It pulls or receives live media over any of eleven ingest transports and serves each stream as any combination of low-latency delivery protocols, from one in-process tokio + axum HTTP origin. Library crate; the `multimux` binary is `multimux-cli`.

```text
  RTSP   ─┐                                          ┌─▶  LL-HLS   (media.m3u8 + parts, fMP4)
  RTP    ─┤                                          ├─▶  DASH     (manifest.mpd, fMP4)
  TS/UDP ─┤                                          ├─▶  LL-DASH  (manifest-ll.mpd, fMP4)
  TS/HTTP─┼─▶  ingest  ─▶  transmux (depay/segment) ──┼─▶  Smooth   (Manifest, fMP4)
  SRT    ─┤         one route = one ingest,           ├─▶  TS-HLS   (media.m3u8, classic .ts)
  HLS-pull┤         one container, N outputs          ├─▶  catch-up (catchup.m3u8, DVR archive)
  DASH-pull┤                                          ├─▶  SRT / RTMP / RTSP push
  Smooth  ─┤                                          └─▶  WHEP     (feature whep)
  RTMP   ─┤
  WHIP   ─┤  (feature whip)
  file   ─┘
```

## What it does / what it does not

**Does:** run many independent routes (one ingest, its served outputs) in one process, so a single instance can serve dozens of unrelated cameras or feeds. Every route is supervised (capped exponential-backoff reconnect), shares one optional output credential across all routes (Basic, Digest, Bearer, reverse-proxy Forwarded, or signed URLs), and can persist finished segments to disk for catch-up and VOD. Ingest once, many outputs: LL-HLS, DASH, LL-DASH and Smooth read the exact same segmented CMAF, with no per-output re-mux. A runtime admin API adds, removes and lists routes and reloads the config without dropping other routes' viewers. Third-party crates can add an input, output or output-auth scheme through a plugin registry without editing this crate.

**Does not:** transcode (samples are opaque and pass through), do per-viewer sessions, server-side ad insertion, manifest rewrites or trick-play, serve both fMP4 and classic TS from one route (the `Trunk` has one segment ring per program: `ts_hls` is mutually exclusive with `llhls`, `dash`, `ll_dash` and `smooth`, and `Config::validate` rejects the mix; run two routes if both are needed), or implement adaptive-bitrate ladders (one route is one rendition). It is not a CDN: put one in front for scale.

## Security defaults

- `Config::allow_open_ingest` is `false`: `Config::validate` refuses an ingest listener that admits a publisher with no authentication (an RTMP route without `stream_key`, a WHIP route, a listener-mode SRT route) on a non-loopback address unless the config sets `"allow_open_ingest": true`. Loopback listeners, RTMP routes with a `stream_key` and caller-mode SRT are unaffected.
- Pull credentials are origin-scoped: HLS, DASH and Smooth pull and TS-over-HTTP credentials are bound to the (scheme, host, port) of the configured source URL; a request to any other origin (an absolute URL on another host, a redirect target) goes without `Authorization`, and a Digest challenge from a redirect target is not answered.
- The admin API is opt-in, binds a listener that must differ from the media listener, and requires its own auth (see below).
- ICE credentials, WHIP/WHEP session ids, `ETag` tokens and RTP SSRCs come from the OS CSPRNG; `Debug` never prints a secret.

## Standards supported

| Standard | Edition | Scope here | Status | Notes |
|---|---|---|---|---|
| IETF HLS 2nd edition `draft-pantos-hls-rfc8216bis` | `-22`, 1 May 2026 (vendored) | LL-HLS output: parts, preload hint, blocking reload (§6.2.5.2), server control; HLS pull input | Partial | No delta updates or rendition reports on output (see `hls-runtime`) |
| IETF RFC 8216 HLS | RFC 8216 (vendored) | Classic TS-segment HLS output (`ts_hls`), catch-up playlists | Full for the served shape | |
| ISO/IEC 23009-1 MPEG-DASH, ETSI TS 103 285 | 23009-1 not vendored (edition unverified); TS 103 285 v1.4.1 vendored | `dash` and `ll_dash` output (live MPD, `availabilityStartTime`, `SegmentTemplate`), DASH pull incl. `SegmentTimeline` | Partial | Chunked-transfer LL-DASH |
| Microsoft MS-SSTR Smooth Streaming | Vendored | Smooth output and pull | Partial | H.264 and AAC only |
| IETF RFC 2326 RTSP 1.0 | RFC 2326 (vendored) | RTSP pull (`rtsp`, `rtsps://`) and RTSP push (ANNOUNCE/RECORD) | Partial | Via `rtsp-runtime` |
| Adobe RTMP 1.0 | Vendored | RTMP publish ingest and RTMP push | Partial | Publish only; via `rtmp-runtime` |
| SRT, `draft-sharabayko-srt-01` | -01 (vendored) | SRT caller and listener ingest, SRT push | Partial | No payload encryption in the adapter |
| IETF RFC 9725 WHIP, `draft-ietf-wish-whep` | RFC 9725; WHEP draft -04 | Feature `whip` ingest (H.264 only), feature `whep` egress, session resources, Trickle ICE, ICE restart | Partial | RFC 8445/5764/3711 via `webrtc-runtime` |
| IETF RFC 3550 RTP, RFC 6184 | As numbered | Raw RTP/UDP ingest with SDP, low-delay H.264 and one-AAC-AU-per-packet depayload, in-order only | Partial | |
| ISO/IEC 13818-1 MPEG-2 TS | 08/2023 (vendored) | TS over UDP, HTTP and SRT; TS output via `ts_hls` and push | Full for carriable codecs | |
| IETF RFC 7617 / 7616 / 6750 / 7235 | Not vendored, unverified here | Pull-side client auth and output-side verify, via `broadcast-auth` | Full | Digest is MD5 `qop=auth` only |

## Inputs x outputs

Every input feeds one `Trunk` per program; every output reads from it, so any input can feed any allowed output set. Limits come from the input's container and codec support, and from the output.

| Input (`type`) | `llhls` / `dash` / `ll_dash` | `smooth` | `ts_hls` | `catchup` | `srt_push` / `rtsp_push` | `rtmp_push` | `whep` |
|---|---|---|---|---|---|---|---|
| `rtsp`, `rtp` | yes | AVC + AAC | yes | with DVR | TS-carriable codecs | AVC + AAC | H.264 |
| `ts_udp`, `ts_http`, `srt` | yes | AVC + AAC | yes | with DVR | TS-carriable codecs | AVC + AAC | H.264 |
| `hls_pull`, `dash_pull`, `smooth_pull` | yes | AVC + AAC | yes | with DVR | TS-carriable codecs | AVC + AAC | H.264 |
| `rtmp` (FLV, AVC + AAC only) | yes | yes | yes | with DVR | yes | yes | H.264 |
| `file` | yes | AVC + AAC | yes | with DVR | TS-carriable codecs | AVC + AAC | H.264 |
| `whip` (H.264 only) | yes | video only | yes | with DVR | yes | video only | H.264 |
| `custom` | per factory | per factory | per factory | with DVR | per factory | per factory | per factory |

`ts_hls` cannot share a route with the fMP4 outputs in the first column or with `smooth`. `catchup` needs `dvr.enabled: true`. The TS-carriable codec set is that of `transmux::TsMux`; the full input and output reference tables follow below.

## Configuration examples

A route is an input plus an optional output list (default `["llhls"]`):

```json
{
  "bind": "0.0.0.0:8080",
  "output_auth": { "scheme": "bearer", "token": "viewer-secret" },
  "routes": [
    { "name": "cam1", "input": { "type": "rtsp", "url": "rtsp://host/stream1" }, "outputs": ["llhls", "dash"] },
    { "name": "ingest", "input": { "type": "rtmp", "listen": "0.0.0.0:1935", "app": "live", "stream_key": "s3cret" },
      "outputs": ["llhls", { "srt_push": { "url": "srt://relay.example:9000" } }] },
    { "name": "slate", "input": { "type": "file", "path": "slate.ts" }, "outputs": ["ts_hls"] }
  ]
}
```

Ready-made configs are under `examples/` on GitHub: a 40-camera fleet, a reverse-proxy `forwarded` setup, many outputs from one ingest, a looping file slate, and `broadcast-origin.json` (eight sources over seven transports, three delivery protocols, signed-URL egress and the admin API, explained in `broadcast-origin.md`).

## Features

| Feature | Default | Description |
|---|---|---|
| `tls` | yes | `rtsps://` sources (rustls through `rtsp-runtime`). |
| `whip` | no | WHIP ingest over HTTP plus ICE and DTLS-SRTP through `webrtc-runtime`'s `media` feature (pulls the ICE/DTLS dependency tree). |
| `whep` | no | WHEP viewer egress (implies `webrtc-runtime/media`, `rtp-packet`). Independent of `whip`. |
| `test-seams` | no | Pre-bound listener hand-in for tests (`Config::prebound`). Not public API. |
| `test-hooks` | no | WebRTC fault injection for tests; implies `test-seams`. Not public API. |

The crate is `std` only.

## Quick start

Load, validate and inspect a config in code (the repository's own example config):

```rust
use multimux::config::{Config, InputSpec};

let config = Config::from_json_file(std::path::Path::new("examples/multi-output.json")).unwrap();
config.validate().unwrap();
assert!(!config.routes.is_empty());
assert!(matches!(config.routes[0].input, InputSpec::Rtsp { .. }));

// Running it: `multimux::serve(config).await` (tokio), or
// `multimux::serve_config_file("routes.json").await` so `POST /admin/reload` works.
```

An open ingest listener on a public address is refused unless you opt in:

```rust
use multimux::config::Config;

let json = r#"{ "bind": "0.0.0.0:8080",
  "routes": [ { "name": "in", "input": { "type": "rtmp", "listen": "0.0.0.0:1935" } } ] }"#;
let config: Config = serde_json::from_str(json).unwrap();
assert!(config.validate().is_err(), "unauthenticated non-loopback RTMP needs allow_open_ingest");
```

From the shell (the `multimux` binary from `multimux-cli`):

```bash
multimux --rtsp rtsp://cam.local/stream --name cam1     # one camera
curl http://0.0.0.0:8080/cam1/master.m3u8
multimux --config routes.json                           # many routes
```

Runnable library examples: `serve_rtsp` and `custom_scheme` (`cargo run -p multimux --example <name>`).

## Reference

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
| `file` | A local media file on disk, identified with `container-probe` and demuxed by the matching `transmux` demuxer | `loop` (default `true`); paced to wall clock, including across the loop point; a long asset holds its parsed samples in memory — see the crate CHANGELOG |
| `whip` | **Push** ingest: binds a `listen` address and accepts WHIP (RFC 9725) publishers; H.264 video only (feature `whip`, ICE + DTLS-SRTP via `webrtc-runtime`) | see [WHIP / WHEP](#whip--whep-session-resources) |

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
unaffected):

| `outputs` token | Served as | Manifest |
| --- | --- | --- |
| `"llhls"` | Low-Latency HLS (RFC 8216bis), fMP4/CMAF | `master.m3u8` + `media.m3u8` (or the configured `playlist_name`) + `iframe.m3u8` |
| `"dash"` | MPEG-DASH, `$Number$`-addressed, fMP4/CMAF | `manifest.mpd` |
| `"ll_dash"` | Low-latency DASH, true chunked-transfer CMAF (whole-segment `$Number$`, served over HTTP chunked transfer while in progress) | `manifest-ll.mpd` |
| `"smooth"` | Microsoft Smooth Streaming (MS-SSTR), fMP4/CMAF | `Manifest` |
| `"ts_hls"` | Classic HLS, whole MPEG-2 TS media segments (RFC 8216 §3, no `#EXT-X-MAP`, no low-latency parts) | `master.m3u8` + `media.m3u8` (or the configured `playlist_name`) |
| `"catchup"` | Catch-up / time-shift / VOD-from-live over the route's DVR archive (requires `dvr.enabled: true` — see [Catch-up / VOD-from-live](#catch-up--vod-from-live)) | `catchup.m3u8` + `vod/p{N}.m3u8` |
| `"srt_push"` | Push re-egress: relay ingested media to a downstream SRT listener, via `srt-runtime` | `url` required |
| `"rtmp_push"` | Push re-egress: relay to a downstream RTMP server (FLV), via `rtmp-runtime` | `url` required |
| `"rtsp_push"` | Push re-egress: relay to a downstream RTSP server (ANNOUNCE/RECORD), via `rtsp-runtime` | `url` required |
| `"whep"` | WHEP (draft-ietf-wish-whep) viewer egress on its own `listen` address (feature `whep`) | see [WHIP / WHEP](#whip--whep-session-resources) |

`"llhls"`/`"dash"`/`"ll_dash"`/`"smooth"` all read the exact same segmented CMAF —
ingest-once, many-outputs, no per-output re-mux — so a route can enable more
than one of them together (e.g. `["llhls", "dash"]`), and different routes
may enable different sets.

**`"ts_hls"` is mutually exclusive with the four fMP4-based outputs on the
same route** — `Config::validate()` rejects, at config-load time, a route
that names both `"ts_hls"` and any of `"llhls"`/`"dash"`/`"ll_dash"`/`"smooth"`.
Container (fMP4 vs. classic MPEG-TS) is a **per-route**, not per-output,
property: the ingest pipeline's `Trunk` has exactly one segment ring per
program, so a program's samples are segmented into fMP4 *or* TS, never both,
without a second ring — a legitimate future want, tracked separately, not
something this release does. If both containers are genuinely needed for one
source today, run two routes against it, one per container.


## Served endpoints

One route ("stream") is served per configured `name`, under `/{stream}/...`:

| Endpoint | Description |
| --- | --- |
| `GET /{stream}/master.m3u8` | Master playlist (if `llhls` or `ts_hls` is enabled). |
| `GET /{stream}/media.m3u8[?_HLS_msn=&_HLS_part=]` | Media playlist (or the configured `playlist_name`) — fMP4/CMAF with LL-HLS parts under `llhls`, whole `.ts` segments with no `#EXT-X-MAP` under `ts_hls`. Blocking Playlist Reload (RFC 8216bis §6.2.5.2) via `_HLS_msn`/`_HLS_part` applies to `llhls`; harmless no-ops (render immediately) under `ts_hls`, which has no low-latency parts to block on. |
| `GET /{stream}/iframe.m3u8[?_HLS_msn=]` | I-frame (trick-play) playlist — LL-HLS only (`llhls`); `404` on `ts_hls` and audio-only origins. Blocking reload via `_HLS_msn` only; `_HLS_part` is rejected with `400`. |
| `GET /{stream}/manifest.mpd` | DASH manifest (if `dash` is enabled). |
| `GET /{stream}/manifest-ll.mpd` | Low-latency DASH manifest (if `ll_dash` is enabled). |
| `GET /{stream}/Manifest` | Smooth Streaming client Manifest XML (if `smooth` is enabled). |
| `GET /{stream}/QualityLevels({bitrate})/Fragments({type}={start time})` | Smooth fragment — the same fMP4 segment bytes as the shared resource route, addressed by Smooth time (if `smooth` is enabled). |
| `GET /{stream}/init-{track}.mp4`, `GET /{stream}/init-{track}-{instance}-{generation}.mp4` | fMP4 init segment (`moov`) — shared across every fMP4-based output (`llhls`/`dash`/`ll_dash`). The bare name is the *current* init (revalidated, `no-cache`); the LL-HLS playlist names the **instance-named** one (the origin's instance token differs for every origin built, across reconnects and restarts), which never changes bytes (`immutable`) — a codec change gets a new generation and an `EXT-X-DISCONTINUITY`. Not served under `ts_hls`: a classic `.ts` segment carries its own PAT/PMT and needs no init segment. |
| `GET /{stream}/seg-{track}-{instance}-{msn}.m4s` (playlist form, `immutable`), `seg-{track}-{msn}.m4s` (DASH/Smooth templates, `max-age=10`) | A full fMP4 media segment: served whole (`Content-Length`) once closed, or streamed over **HTTP chunked transfer-encoding** while still in progress (issue #721 — `ll_dash`'s low-latency delivery). |
| `GET /{stream}/seg-{track}-{seq}.ts` | A full whole-packet MPEG-2 TS media segment (`ts_hls` only) — self-contained (its own PAT + PMT, exactly one program per RFC 8216bis §3.1.1), served whole once closed. |
| `GET /{stream}/part-{track}-{seq}.{part}.m4s` | An LL-HLS partial segment of the in-progress segment (also how `ll_dash`'s chunked-transfer path internally sources the bytes it streams — never addressed directly by the LL-DASH MPD itself). Not applicable under `ts_hls`, which never produces parts. |
| `GET /healthz` | Liveness — always `200`. Never gated by `output_auth`. |
| `GET /readyz` | Readiness — `200` once at least one route is live, `503` otherwise. Never gated by `output_auth`. |
| `GET /metrics` | Prometheus metrics. Never gated by `output_auth`. |

An unknown stream name, or a filename `multimux` doesn't recognize, returns
`404`.


## WHIP / WHEP session resources

With the `whip` / `whep` Cargo features, a route can accept a WebRTC publisher
(`POST /whip`) or serve viewers (`POST /whep`). Sessions are real resources
(RFC 9725 sections 4.2, 4.3, 8 and draft-ietf-wish-whep):

- The `201 Created` `Location` is a per-session resource,
  `/whip/session/<id>` / `/whep/session/<id>` (a 32-character unguessable id
  from the OS CSPRNG); the id-less paths no longer exist (`404`). Clients must
  follow `Location` rather than hard-code a path. Both `201`s carry the
  `ETag` the state machine owns.
- `DELETE` on the resource ends the session (`200`): the media task drops the
  transport (WHIP also releases the published program), the `Trunk` cursor
  (WHEP) and the capacity slot; a repeated `DELETE`, or one for a session that
  already ended, is `404`.
- `PATCH` carries Trickle ICE (`application/trickle-ice-sdpfrag`, optional
  `If-Match`): candidates are added to the live transport (`204`); a wrong
  `Content-Type` is `415`, an `If-Match` naming another entity-tag `412`, an
  unparsable fragment / one for another ICE generation / with nothing to add
  `400`, an unknown resource `404`.
- `If-Match: *` is an **ICE restart** (RFC 9725 section 4.3.2): the PATCH
  carries the client's new `ice-ufrag`/`ice-pwd`; the server restarts ICE in
  place under new credentials of its own (DTLS/SRTP untouched, media keeps
  flowing) and answers `200` with a new `ETag` and an sdpfrag holding its new
  credentials and candidates. A restart fragment that keeps the ufrag or lacks
  a pwd is `400`; afterwards the old `ETag` is `412`.


## DASH timeline

`MPD@availabilityStartTime` is the program's **presentation origin** (the
instant its `Trunk` was minted, `RouteHandle::presentation_origin(program)`),
not the route's construction time, so `AST + (N - startNumber) * duration`
holds across a reconnect that restarts at segment 1 (ISO/IEC 23009-1 5.3.1.2,
5.3.9.5.3); before a program is published the MPD answers `503`. A live
`SegmentTimeline` (`$Time$`) MPD refresh in DASH pull is extended by time
(`S@t` is absolute), and a negative `S@r` is resolved to the next `@t`/Period
end; tests use real DASH-IF livesim2 MPDs. The `xs:duration` spelling is the one
shared `transmux::dash::xs_duration`.


## Catch-up / VOD-from-live

`"catchup"` (`outputs`, above) serves the route's durable DVR archive
(`routes.dvr`) for time-shift, a catch-up window, and finished-programme
VOD — a route must have `routes.dvr.enabled: true` (and at least one
retention limit) for it to have anything to serve; `Config::validate()`
rejects a route naming `"catchup"` without it.

| Endpoint | Description |
| --- | --- |
| `GET /{stream}/catchup.m3u8[?window_secs=N]` | A live-continuing playlist spanning the archive plus whatever the live `Trunk` has closed since the archive was last written — one continuous, gap-free sequence across that boundary, never two disjoint playlists. `window_secs` bounds it to the trailing N seconds; omitted, the whole archive plus the live tail. |
| `GET /{stream}/vod/p{N}.m3u8` | One archived period's segments as a standalone playlist — `#EXT-X-ENDLIST` + `PLAYLIST-TYPE:VOD` once that period is definitively finished (a later period exists on disk), otherwise the same still-growing shape as `catchup.m3u8` restricted to that period. |
| `GET /{stream}/catchup/seg-{seq}.{ext}` | The segment bytes either playlist above references — read straight off disk when archived, or resolved through the same live `HlsOrigin` every other output shares when not (the still-unarchived tail). |

Shares `output_auth` with every other output — no separate credential or
auth path for catch-up.


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
`"bearer"` (token), `"forwarded"` (see below), `"signed_url"` (CDN-style HMAC-SHA256 query tokens with key rotation across `kid`s, so a player needs no `Authorization` header) and `"custom"` (a registered plugin scheme).

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

### DVR recording

A route can persist finished segments to disk for catch-up / VOD:

```json
{
  "routes": [
    {
      "name": "cam1",
      "input": { "type": "rtsp", "url": "rtsp://host/stream" },
      "dvr": {
        "enabled": true,
        "archive_root": "/data/dvr",
        "period_duration_secs": 10800,
        "retention_periods": 8,
        "retention_bytes": 0,
        "overrun": "gap"
      }
    }
  ]
}
```

- `enabled` (default `false`): opt-in.
- `archive_root`: filesystem directory; one subdirectory per route.
- `period_duration_secs` (default **10800** = 3 hours): a new period
  container file is started when this much wall-clock time elapses.
  The default keeps a feature film in one file. Retention quantises to
  the period — a truncation costs up to one period.
- `retention_periods`: keep at most this many period files (0 = unlimited).
- `retention_bytes`: keep at most this many total bytes (0 = unlimited).
  At least one retention axis must be non-zero when DVR is enabled.
- `overrun` (default `"gap"`): what happens when the live ring evicts a
  segment before the recorder consumes it:
  - `"gap"` — the recording gets a hole; live ingest is unaffected.
    **The default.**
  - `"stall"` — publication blocks until the recorder catches up.
    The archive is lossless, but can stall live output.
  - `"terminate"` — the recorder's pin is dropped; recording stops and
    existing files are kept.

#### On-disk layout

```text
<archive_root>/<route>/
├── p0.m4s          ← period 0 container file (init at head, then fragments)
├── p0.idx          ← JSON byte-range index: (seq, pts_ns, offset, len)
├── p1.m4s
├── p1.idx
├── …
```

For **fMP4**, the init segment is written at the head of each period file
and media fragments are appended — the file is a valid CMAF track (init +
concatenated fragments). For **TS**, segments are natively concatenable
188-byte packets and the file is directly playable with no init needed.

The index sidecar (`pN.idx`) maps every segment to a byte range
(`seq`, `start_pts_ns`, `byte_offset`, `byte_len`) — issue #900 will serve
`EXT-X-BYTERANGE` directly from these offsets. The index is flush-as-you-go
and can be **rebuilt by rescanning** the period file (crash recovery).

A new period is rolled on either duration expiry OR when the fMP4 init
segment changes mid-recording (mid-stream track addition — issue #781).

Recording is a `media_plane::egress::SegmentEgress` implementation draining
its own pinning `SegmentCursor` — it never holds a lock the live-serving
path needs and never perturbs live output.

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
See `examples/custom_scheme.rs` for a
complete, runnable example.


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
- **Mid-GOP-safe push** — RTMP/SRT/RTSP push drops every sample until the
  first video sync sample after a (re)connect, so a downstream never starts
  mid-GOP; an RTSP interleaved frame whose RTP payload does not parse is
  skipped, logged and counted (`malformed_media_frames`) rather than ending the
  ingest session.
- **Graceful shutdown** — Ctrl-C / `SIGTERM` drains in-flight requests and
  every route's ingest task before exiting.


## v1 limits (still out of scope)

- Per-viewer sessions, server-side ad insertion, manifest rewrites, trick-play, ABR ladders.
- Limits inherited from `transmux`'s `RtpStreamDepacketiser` (raw RTP input): low-delay H.264 only (no B-frame reordering), one AAC access unit per RTP packet, packets must arrive in order.
- WHIP ingest is H.264 video only.

## Design notes

One ingest architecture for every input: `media-plane`'s `Dialer` (the inputs that dial out) or `Listener` (RTMP, WHIP push accept) driven by `IngestDriver` / `ListenDriver`, publishing into a per-program `Trunk` that every egress serves from, so there is no second path a sample can take from socket to segment. HTTP outputs resolve through `hls-runtime`'s `HlsOrigin` and the `ServedEgress` trait; the DVR recorder is a `SegmentEgress` over its own pinning cursor, so it never perturbs live output. Errors are structured (`MultimuxError`), secrets are redacted everywhere, and the config schema is guarded by `tests/example_configs.rs` so the example configs cannot drift. The design documents are `docs/superpowers/specs/2026-07-18-multimux-hub-design.md` and (for the original RTSP-only v1) `2026-07-14-multimux-design.md` in the workspace.

## Related crates

`multimux-cli` (the `multimux` binary). Built on `transmux` (depayload, segmentation, DASH and Smooth packaging), `media-plane` (ingest drivers, `Trunk`, egress shapes), `hls-runtime` (LL-HLS origin and HLS pull), `rtsp-runtime`, `rtmp-runtime`, `srt-runtime`, `webrtc-runtime` (features `whip`, `whep`), `broadcast-auth`, `broadcast-hls`, `container-probe` (file input detection) and `rtp-packet`.

## MSRV and licence

MSRV 1.95.0. Licensed under MIT OR Apache-2.0.
