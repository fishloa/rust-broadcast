# Changelog

All notable changes to `multimux-cli` will be documented in this file.

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
