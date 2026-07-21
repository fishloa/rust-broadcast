# multimux-cli

CLI for `multimux` — a multi-input (RTSP/RTP/TS-UDP/TS-HTTP/HLS-pull),
multi-output (LL-HLS/DASH/LL-DASH) just-in-time repackaging HTTP origin
library.

## Installation

```bash
cargo install multimux-cli
```

## Usage

Two usage forms:

### Single-route quick start

```bash
multimux --rtsp rtsp://cam.local/stream --name cam1
```

Pulls a single live RTSP source and serves it as LL-HLS at `http://localhost:8080/cam1/media.m3u8`.
Add `--dash` (or `--outputs llhls,dash`) to serve DASH from the same ingest
too. This quick-start form only reaches RTSP input with no client-side
auth — every other input transport, per-route output selection, and shared
output auth needs a config file.

### Multi-route configuration file

```bash
multimux --config routes.json
```

Point to a JSON file describing any number of routes — each an independent
input (RTSP/RTP/TS-UDP/TS-HTTP/HLS-pull, with optional client-side auth) to
any set of outputs (LL-HLS/DASH/LL-DASH) — plus segmentation, window, bind,
timeout, and shared output-auth parameters. A minimal two-camera example:

```json
{
  "bind": "0.0.0.0:8080",
  "routes": [
    { "name": "cam1", "input": { "type": "rtsp", "url": "rtsp://host/stream1" } },
    { "name": "cam2", "input": { "type": "rtsp", "url": "rtsp://host/stream2" }, "outputs": ["llhls", "dash"] }
  ]
}
```

See the `multimux` crate README for the full config schema (every input/
output/output-auth shape, the 40-camera shared-output-auth scenario, and
the reverse-proxy deployment) and served endpoint table.

## Flags

- `--config <FILE>` — JSON config file (conflicts with `--rtsp`/`--name`/`--outputs`/`--dash`)
- `--rtsp <URL>` — RTSP source URL (requires `--name`)
- `--name <NAME>` — served stream name/URL path (requires `--rtsp`)
- `--outputs <LIST>` — comma-separated output kinds for the quick start (`llhls`, `dash`; default `llhls`)
- `--dash` — shorthand for `--outputs llhls,dash`
- `--bind <ADDR>` — HTTP origin bind address (default: `127.0.0.1:8080`)
- `--target-duration <SECS>` — full-segment target duration (default: `4.0`)
- `--part-ms <MS>` — LL-HLS part target in milliseconds (default: `500`)
- `--window <N>` — rolling window depth: full segments in RAM (default: `3`)

Set `RUST_LOG` (`EnvFilter` syntax, e.g. `RUST_LOG=multimux=debug`) to
control log verbosity; defaults to `info`.

For the served endpoint table and full configuration schema, see the [`multimux`](https://crates.io/crates/multimux) crate.
