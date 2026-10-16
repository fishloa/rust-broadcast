# multimux-cli

[![crates.io](https://img.shields.io/crates/v/multimux-cli.svg)](https://crates.io/crates/multimux-cli)

The `multimux` command: a multi-input, multi-output just-in-time repackaging HTTP origin. It is a thin front end over the `multimux` library crate, which holds the engine, the config schema and the full reference.

## What it does / what it does not

**Does:** parse flags, build a `multimux::config::Config`, and run the origin until Ctrl-C or `SIGTERM`, which drains in-flight requests and every route's ingest task. Two forms: a single-route quick start (one RTSP camera, optionally with SRT, RTMP or RTSP push relays), and `--config <FILE>` for any number of routes across every input and output the library supports. Log verbosity comes from `RUST_LOG` (`EnvFilter` syntax, default `info`).

**Does not:** reach any input other than RTSP from flags, select outputs beyond `llhls` and `dash` from flags, set shared output auth, DVR, or the admin API from flags, or transcode (samples are opaque). Everything else needs a JSON config file. The origin does no standards work itself; see the `multimux` crate for the standards table, the inputs and outputs matrix and the security defaults.

## Standards supported

None of its own. The binary delegates to `multimux`, which serves LL-HLS (draft-pantos-hls-rfc8216bis), classic HLS (RFC 8216), MPEG-DASH, Smooth Streaming, and ingests RTSP (RFC 2326), RTMP, SRT, WHIP (RFC 9725) and more. The `multimux` crate README carries the per-standard table with editions and status.

## Features

None. The binary builds `multimux` with its default `tls` feature (`rtsps://` sources). WHIP and WHEP are library features (`whip`, `whep`) that this binary does not enable.

## Install and usage

```bash
cargo install multimux-cli      # installs the `multimux` binary
```

`multimux --help`:

```text
Usage: multimux [OPTIONS]

Options:
      --config <FILE>          JSON config file describing routes + segmentation/window/bind parameters
      --rtsp <URL>             Single-route quick start: RTSP source URL to pull (requires --name)
      --name <NAME>            Single-route quick start: served stream name, i.e. the URL path segment (requires --rtsp)
      --bind <ADDR>            host:port the HTTP origin binds [default: 0.0.0.0:8080]
      --target-duration <SECS> Target full-segment duration, in seconds [default: 4]
      --part-ms <MS>           LL-HLS part target, in milliseconds [default: 500]
      --window <N>             Rolling window depth: full segments retained in RAM [default: 8]
      --outputs <LIST>         Quick start: delivery protocol(s), comma-separated (llhls, dash) [default: llhls]
      --dash                   Quick start shorthand for --outputs llhls,dash
      --srt-push <URL>         Push to a remote SRT Listener (Caller mode). Repeatable
      --rtmp-push <URL>        Push to a remote RTMP server (client publish). Repeatable
      --rtsp-push <URL>        Push to a remote RTSP server (ANNOUNCE/RECORD). Repeatable
  -h, --help                   Print help
  -V, --version                Print version
```

`--config` conflicts with `--rtsp`, `--name`, `--outputs` and `--dash`.

### 1. Single camera, LL-HLS

```bash
multimux --rtsp rtsp://cam.local/stream --name cam1
curl http://localhost:8080/cam1/master.m3u8
```

Serves `http://localhost:8080/cam1/media.m3u8` (LL-HLS). Add `--dash` to also serve `http://localhost:8080/cam1/manifest.mpd` from the same ingest.

### 2. Many routes from a config file

```bash
multimux --config routes.json
```

```json
{
  "bind": "0.0.0.0:8080",
  "routes": [
    { "name": "cam1", "input": { "type": "rtsp", "url": "rtsp://host/stream1" } },
    { "name": "cam2", "input": { "type": "rtsp", "url": "rtsp://host/stream2" }, "outputs": ["llhls", "dash"] }
  ]
}
```

### 3. A local file as a live source (no camera needed)

Verified against the committed capture `fixtures/ts/h264_aac.ts`:

```bash
cat > slate.json <<'EOF'
{ "bind": "127.0.0.1:18080",
  "routes": [ { "name": "slate",
                "input": { "type": "file", "path": "fixtures/ts/h264_aac.ts" },
                "outputs": ["llhls", "dash"] } ] }
EOF
multimux --config slate.json &
curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:18080/healthz     # 200
curl -s http://127.0.0.1:18080/slate/master.m3u8                            # #EXTM3U ... media.m3u8
curl -s -o /dev/null -w "%{http_code}\n" http://127.0.0.1:18080/slate/manifest.mpd   # 200
```

The container is identified with `container-probe`, so the file may be TS, fMP4, progressive MP4, Matroska/WebM, MPEG-PS or FLV; the route loops by default.

More ready-made configs live in `multimux/examples/` (a 40-camera fleet, a reverse-proxy `forwarded` setup, many outputs from one ingest, signed-URL egress with an admin API).

## Design notes

Uses `clap` derive with named flags per the workspace CLI standard, and `tracing-subscriber` for logs. The quick-start flags cannot express anything the config file cannot, so the config is the source of truth. The binary is `doc = false` so rustdoc does not race the `multimux` library docs; its interface is documented by `--help`.

## Related crates

`multimux` (the library and the full reference), which is built on `rtsp-runtime`, `rtmp-runtime`, `srt-runtime`, `hls-runtime`, `media-plane`, `transmux` and `broadcast-auth`.

## MSRV and licence

MSRV 1.95.0. Licensed under MIT OR Apache-2.0.
