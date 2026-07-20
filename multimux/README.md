# multimux — live RTSP → LL-HLS just-in-time repackaging origin

A thin **client + server wrap** around two existing workspace crates —
`rtsp-runtime` (RTSP pull) and `transmux` (RTP depayload + LL-HLS CMAF
segmentation) — that pulls one or more live RTSP sources and serves each as
**Low-Latency HLS** (RFC 8216bis, `#EXT-X-PART` + blocking playlist reload)
from an in-process tokio + axum HTTP origin. Muxing only: samples are opaque
and are never transcoded.

## Quick start

Single-route quick start (one camera, no config file):

```bash
multimux --rtsp rtsp://cam.local/stream --name cam1
curl http://0.0.0.0:8080/cam1/master.m3u8
```

Multi-route JSON config file:

```json
{
  "bind": "0.0.0.0:8080",
  "target_duration_secs": 4.0,
  "part_target_ms": 500,
  "window_segments": 8,
  "routes": [
    { "name": "cam1", "rtsp_url": "rtsp://host/stream1" },
    { "name": "cam2", "rtsp_url": "rtsp://host/stream2" }
  ]
}
```

```bash
multimux --config routes.json
```

Every flag/field has a default (see `multimux --help` or
`multimux::config::Config::default()`); `--rtsp`/`--name` and `--config` are
mutually exclusive — pass one or the other.

## Served endpoints

One route ("stream") is served per configured `name`, under `/{stream}/...`:

| Endpoint | Description |
| --- | --- |
| `GET /{stream}/master.m3u8` | Minimal single-variant HLS master playlist. |
| `GET /{stream}/media.m3u8[?_HLS_msn=&_HLS_part=]` | LL-HLS media playlist. With `_HLS_msn`/`_HLS_part` present, blocks (bounded) until that segment/part is available — RFC 8216bis §6.2.5.2 Blocking Playlist Reload. |
| `GET /{stream}/init-{track}.mp4` | fMP4 init segment (`moov`). |
| `GET /{stream}/seg-{track}-{seq}.m4s` | A closed full media segment. |
| `GET /{stream}/part-{track}-{seq}.{part}.m4s` | An LL-HLS part of the in-progress segment. |

An unknown stream name, or a filename `multimux` doesn't recognize, returns
`404`.

## v1 scope and limits

multimux wraps already-authored library crates and carries no media/transport
spec logic of its own — any missing RTP/RTSP/SDP/codec capability is a library
gap fixed upstream, in `transmux` or `rtsp-runtime`, never in this crate. See
[`docs/superpowers/specs/2026-07-14-multimux-design.md`](../docs/superpowers/specs/2026-07-14-multimux-design.md)
in the workspace root for the full design.

In scope (v1):
- Ingest: RTSP **pull** only, interleaved RTP/RTCP over TCP.
- Codecs: H.264 video + AAC audio.
- Output: **LL-HLS only** — one global output protocol per instance.
- Routing: N independent `rtsp_url -> stream name` routes per instance.
- Config: CLI-first (clap), with an optional JSON config file for multiple
  routes.
- Serving: standalone tokio + axum listener; an in-RAM rolling window per
  stream (N full segments + the in-progress segment's live parts), oldest
  segment evicted on roll. Live-only — nothing is retained after eviction.

Explicitly out of scope for v1 (deferred):
- **DASH / LL-DASH** — planned as v1.1; `transmux`'s DASH packager is already
  built, multimux just doesn't wire it up yet.
- SRT/TS/file inputs (RTSP pull only).
- Per-viewer sessions, server-side ad insertion, manifest rewrites.
- DVR / VOD / disk spill (the window is RAM-only and rolls forward).
- TLS + auth (run a reverse proxy in front for either).
- Trick-play.

Additional documented limits inherited from the underlying streaming
depayloader (`transmux`'s `RtpStreamDepacketiser`, issue #700): low-delay
H.264 only (no B-frame reordering), one AAC access unit per RTP packet, and
packets must arrive in order — all true of a direct RTSP pull over TCP.

## Examples

```bash
# Serve a synthetic stream with no camera / network required.
cargo run --example serve_mock

# Serve one real RTSP source.
cargo run --example serve_rtsp -- rtsp://cam.local/stream
```

## Spec

RFC 8216bis (HTTP Live Streaming, 2nd edition) — Low-Latency HLS: `#EXT-X-PART`
(§4.4.4.9), `#EXT-X-PART-INF`/`#EXT-X-SERVER-CONTROL` (§4.4.3.7/§4.4.3.8), and
Blocking Playlist Reload (§6.2.5.2). RTSP 1.0 (RFC 2326) for ingest, via
`rtsp-runtime`.

## License

MIT OR Apache-2.0
