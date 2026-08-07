# Changelog

All notable changes to `multimux-cli` will be documented in this file.

## [Unreleased]

### Added
- `--srt-push <URL>`, `--rtmp-push <URL>`, `--rtsp-push <URL>` — push
  outputs for relaying ingested media to downstream servers (#744).

## [0.5.0] - 2026-08-05

### Changed

- Requires `multimux` 0.7, which gained MPTS ingest (#906), mid-stream track
  additions (#781), Smooth Streaming output (#742), and DVR archive (#746).
  **No change in this crate itself** — the bump propagates `multimux`'s
  pre-1.0 caret boundary (`^0.6` -> `^0.7`) so a consumer cannot end up with
  two `multimux` copies in one graph.

## [0.4.0] - 2026-08-02

### Changed

- Requires `multimux` 0.6, which gained the runtime admin API (#749),
  signed-URL egress auth (#747) and classic MPEG-TS HLS output (#887).
  **No change in this crate itself** — the bump propagates `multimux`'s
  pre-1.0 caret boundary (`^0.5` -> `^0.6`) so a consumer cannot end up with
  two `multimux` copies in one graph.

## [0.3.1] - 2026-07-30

### Fixed
- Floor `multimux` to `0.5.1`. The `^0.5` bucket also contains 0.5.0,
  which is built against `media-plane` 0.1.0, so a consumer could resolve
  two `transmux` minors into one graph and hit trait-resolution errors
  pointing at this crate's internals (#858).

## [0.3.0] - 2026-07-28

## [0.2.1] - 2026-07-26

### Changed
- Bump the `multimux` dependency to 0.4 (adds the RTMP push ingest input; no
  CLI surface change).

## [0.2.0] - 2026-07-21

### Added
- `--outputs <LIST>` — comma-separated delivery protocol(s) for the
  single-route quick start (`llhls`, `dash`; defaults to `llhls`, preserving
  existing invocations unchanged), and a `--dash` shorthand for `--outputs
  llhls,dash` (issue #663 P4 "ingest-once, many-outputs"). Ignored when
  `--config` is used — a config file sets `outputs` per route.
- `tracing-subscriber` process-wide subscriber init (`fmt` + `EnvFilter`,
  `RUST_LOG`-overridable, default `info`, written to stderr): the `multimux`
  library only ever emits `tracing` events and never installs a subscriber
  itself, so the CLI now owns that (the top-level fatal-error report stays a
  plain `eprintln!` so it is never swallowed by a log filter).

### Changed
- Depends on `multimux` 0.3 (config-driven multi-input/multi-output hub, was
  the RTSP-pull/LL-HLS-only 0.2): the single-route quick start now builds a
  `multimux::config::InputSpec::Rtsp` (with no config-supplied `auth`) rather
  than the old flat `rtsp_url` field. A CLI-invalid config now reports via
  `MultimuxError::ConfigInvalid { field, reason }` instead of the old
  stringly `MultimuxError::Config`.

## [0.1.0] - 2026-07-16

### Added
- Initial release: the `multimux` CLI binary, extracted from the `multimux`
  crate (which is now a library). `--config <FILE>` (JSON routes) or the
  single-route quick start `--rtsp <URL> --name <NAME>`, plus `--bind`,
  `--target-duration`, `--part-ms`, `--window`.
