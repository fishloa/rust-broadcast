# multimux-cli

CLI for the `multimux` live RTSP → LL-HLS just-in-time repackaging HTTP origin library.

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

Pulls a single live RTSP source and serves it as LL-HLS at `http://localhost:8080/cam1/playlist.m3u8`.

### Multi-route configuration file

```bash
multimux --config routes.json
```

Point to a JSON file describing multiple routes, segmentation, window, and bind parameters. See the `multimux` crate README for the served endpoint table and API scope.

## Flags

- `--config <FILE>` — JSON config file (conflicts with `--rtsp`/`--name`)
- `--rtsp <URL>` — RTSP source URL (requires `--name`)
- `--name <NAME>` — served stream name/URL path (requires `--rtsp`)
- `--bind <ADDR>` — HTTP origin bind address (default: `127.0.0.1:8080`)
- `--target-duration <SECS>` — full-segment target duration (default: `4.0`)
- `--part-ms <MS>` — LL-HLS part target in milliseconds (default: `500`)
- `--window <N>` — rolling window depth: full segments in RAM (default: `3`)

For the served endpoint table and configuration schema, see the [`multimux`](https://crates.io/crates/multimux) crate.
