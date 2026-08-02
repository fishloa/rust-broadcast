# A complex HLS origin, explained

Companion to [`broadcast-origin.json`](broadcast-origin.json) — eight sources across six ingest
transports, packaged to three delivery protocols, with signed-URL egress and a runtime admin
API.

```
multimux --config multimux/examples/broadcast-origin.json
```

The config is short. What follows is why the values are what they are, because most of the ways
this goes wrong are configuration choices rather than bugs.

## Latency budget

```json
"target_duration_secs": 4.0,
"part_target_ms": 500,
"window_segments": 12
```

`part_target_ms` is what actually determines low-latency glass-to-glass; `target_duration_secs`
mostly governs how often a non-LL client reloads.

**A part cannot be shorter than the encoder's GOP.** Parts are cut on frame boundaries, and a
part that would land mid-GOP is not independently decodable. With a 500 ms part target and a 2 s
GOP you get parts every 2 s regardless of what you asked for — the config will look wrong when
the encoder is what needs changing.

`window_segments: 12` at 4 s is a 48-second playlist window. Larger costs memory in the `Trunk`
ring and lets a client rewind further; smaller risks a slow client falling off the back and
getting a `Lagged` error rather than media.

RFC 8216bis §4.4.3.8 requires `PART-HOLD-BACK >= 2×` the part target and recommends `>= 3×`.
`hls-runtime` derives it at 3× — you do not configure it, and it is deliberately not exposed.

## Ingest — six transports

| route | transport | notes |
|---|---|---|
| `studio-a` | RTSP | plain |
| `studio-b` | RTSP over TLS | `rtsps://` |
| `contribution-srt` | SRT listener | `listen: 0.0.0.0:9000` — caller dials in. Use `remote` instead to dial out |
| `encoder-rtmp` | RTMP listener | `listen: 0.0.0.0:1935`, `app: live` — encoder publishes in |
| `satellite-mux` | MPEG-TS over UDP multicast | `addr` is the local bind, `multicast_group` the group to join |
| `partner-feed` | MPEG-TS over HTTP | |
| `upstream-hls` | HLS pull | re-packages someone else's HLS |
| `upstream-dash` | DASH pull | |

**Note the direction difference.** RTSP, TS-HTTP, HLS-pull and DASH-pull *dial out*; SRT and
RTMP *listen*. The listening ones bind ports on this host — check they do not collide with each
other or with `bind`/`admin.bind`.

**Multicast needs the host to be on that group.** `addr` is the local bind and
`multicast_group` is the group joined; `239.10.0.5` produces nothing if IGMP membership is not
established on the right interface. It is not a config error, and there
is no error message; the route just stays empty.

## Outputs

`llhls`, `dash`, `ll_dash` — per route, since not every source justifies every packaging.

Serving both `llhls` and `ll_dash` from one ingest costs one segmentation, not two: they share
the `Trunk`. Adding `dash` alongside `ll_dash` is for clients that cannot do low-latency DASH.

Classic **TS-HLS output is not yet available** here. `hls-runtime` supports it
(`Container::MpegTs`), but multimux's pipeline is fMP4-only end to end — see issue #887, which
scopes what it needs.

## Egress auth — signed URLs

```json
"output_auth": { "scheme": "signed_url", "keys": [...] }
```

HMAC-SHA256 over `<path>\n<exp>\n<ip-or-empty>`, with the signature in the URL:

```
?exp=<unix-seconds>&kid=<key-id>&sig=<base64url-nopad>[&ip=<addr>]
```

**The path is signed.** Without that, a token minted for one route replays against every other
route on the origin, which defeats the whole mechanism.

**Two keys are listed on purpose.** `kid` selects the key, so you can publish a new key,
let it become current, and retire the old one only once every URL signed under it has expired.
One key means rotation invalidates every live URL simultaneously.

Secrets must be **at least 32 bytes**, enforced at config load rather than per request — a short
key fails at startup, not at 3 a.m. under load.

`exp` is absolute Unix seconds with no clock-skew grace. Pick the window when minting: long
enough to cover a segment fetch and a retry, short enough that a leaked URL expires.

Alternatives if signed URLs do not fit: `basic`, `digest`, `bearer`, or `forwarded` for when a
reverse proxy has already authenticated (see [`reverse-proxy.json`](reverse-proxy.json)).

## Admin API

```json
"admin": { "bind": "127.0.0.1:8081", "auth": { "scheme": "bearer", ... } }
```

**Bound to loopback, not `0.0.0.0`.** This listener can add and remove routes. Exposing it to the
network is close to handing out remote code execution, so it deliberately binds separately from
`bind` and its `auth` field is **mandatory** — not `Option` — so an unauthenticated admin API is
impossible to construct rather than merely discouraged.

In production, put it on a management interface or behind a bastion.

```
GET    /admin/routes           list routes and status
GET    /admin/routes/{name}    one route
POST   /admin/routes           add (body: the same Route shape as this file)
DELETE /admin/routes/{name}    remove, draining cleanly
POST   /admin/reload           re-read this file and converge
```

`POST` on an existing name returns **409** rather than silently replacing — losing a live channel
to a typo'd re-`POST` is the kind of accident that only happens once but is very expensive.

`POST /admin/reload` converges: routes added, removed, and changed. **A route whose config has
not changed is not restarted** — bouncing untouched channels on reload is the classic failure of
this feature, and there is a test asserting a third route's identity is bit-identical across a
reload.

Omit the `admin` block entirely and there is no admin API at all. A control plane that defaults
to on is a footgun.

## Operating it

```bash
# list
curl -H 'Authorization: Bearer $TOKEN' http://127.0.0.1:8081/admin/routes

# add a camera without restarting — other routes keep serving
curl -X POST -H 'Authorization: Bearer $TOKEN' -H 'Content-Type: application/json' \
  -d '{"name":"cam-12","input":{"type":"rtsp","url":"rtsp://cam12.internal/main"},
       "outputs":["llhls"]}' \
  http://127.0.0.1:8081/admin/routes

# remove it, draining in-flight viewers
curl -X DELETE -H 'Authorization: Bearer $TOKEN' \
  http://127.0.0.1:8081/admin/routes/cam-12
```

`GET /metrics` exposes Prometheus metrics; `GET /healthz` and `GET /readyz` are the liveness and
readiness probes. Those sit on the **media** listener, since they describe media serving.

## Scaling note

Writer cost in the `Trunk` is O(N) in **cursor** count, and a cursor is per distinct consumer —
not per peer. Ten thousand viewers on one route is one cursor, not ten thousand. Adding routes
costs memory (one ring set each); adding viewers to an existing route is close to free.

For a much larger fleet see [`webcam-fleet-40.json`](webcam-fleet-40.json).

## These values are tested, not illustrative

`multimux/tests/example_configs.rs` deserialises this file as a real `Config` and runs
`validate()` on it — the same two steps `multimux --config` performs at startup. It also asserts
the breadth (seven ingest transports, three outputs, egress auth present) and that the admin
listener is on loopback and not sharing the media port.

That guard is not ceremony. Writing this example produced three plausible-looking schema errors
that the test caught: a `_comment` block (`Config` uses `deny_unknown_fields`), `srt: { url }`
(it is `listen` or `remote`), and `ts_udp: { url }` (it is `addr` plus `multicast_group`). A
config example that does not parse is worse than none — the reader assumes their setup is wrong
rather than the documentation.
