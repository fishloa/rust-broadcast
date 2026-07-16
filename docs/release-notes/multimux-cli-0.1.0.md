# multimux-cli 0.1.0 — 2026-07-16

**New crate: the `multimux` CLI binary.** The `multimux` command-line tool
(RTSP → LL-HLS just-in-time repackaging HTTP origin) has been extracted into a
dedicated `multimux-cli` crate. This crate contains the clap CLI and binary,
depending on the `multimux` library (0.2.0) for the serving logic.

## Installation

```bash
cargo install multimux-cli
```

This installs the `multimux` binary.

## Usage

### Quick start: single route

```bash
multimux --rtsp rtsp://cam.local/stream --name cam1
```

Serves LL-HLS at `http://127.0.0.1:8080/cam1.m3u8`.

### Config file: multiple routes

Create `routes.json`:

```json
{
  "bind": "0.0.0.0:8080",
  "target_duration_secs": 4,
  "part_target_ms": 500,
  "window_segments": 10,
  "routes": [
    { "name": "cam1", "rtsp_url": "rtsp://cam1.local/stream" },
    { "name": "cam2", "rtsp_url": "rtsp://cam2.local/stream" }
  ]
}
```

Then:

```bash
multimux --config routes.json
```

## Flags

- `--config FILE` — JSON routes and segmentation config.
- `--rtsp URL` — RTSP source (quick start; requires `--name`).
- `--name NAME` — stream name / URL path segment (quick start; requires `--rtsp`).
- `--bind ADDR` — HTTP origin bind address (default: `127.0.0.1:8080`).
- `--target-duration SECS` — full-segment duration target (default: 4.0).
- `--part-ms MS` — LL-HLS part target in milliseconds (default: 500).
- `--window N` — rolling window depth: full segments in RAM (default: 10).

For more, see the `multimux` library documentation for the served endpoint table
and v1 limitations.

## Dependencies

- `multimux` (0.2.0) — the LL-HLS origin library.
- `tokio` — async runtime.
- `clap` — CLI argument parsing.
