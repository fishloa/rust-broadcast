# multimux-cli 0.2.0 — 2026-07-21

Additive minor, following `multimux` 0.3.0's multi-input/multi-output hub
restructure (issue #663).

## Added

- `--outputs <LIST>` — comma-separated delivery protocol(s) for the
  single-route quick start (`llhls`, `dash`; defaults to `llhls`, so an
  existing invocation is unaffected), plus a `--dash` shorthand for
  `--outputs llhls,dash` ("ingest-once, many-outputs"). Ignored when
  `--config` is used — a config file sets `outputs` per route.
- Process-wide `tracing` subscriber init (`RUST_LOG`-overridable, default
  `info`, stderr): the `multimux` library only ever emits `tracing` events
  and never installs a subscriber itself, so the CLI now owns that.

## Changed

- Depends on `multimux` 0.3 — the quick start's internal config-building now
  targets the new `InputSpec::Rtsp` shape; a CLI-invalid config now reports
  via the new `MultimuxError::ConfigInvalid { field, reason }`.

## Compatibility

No CLI flag was removed or changed meaning — every existing invocation
(`--config`, `--rtsp`/`--name`, `--bind`/`--target-duration`/`--part-ms`/
`--window`) behaves identically. MSRV 1.86.
