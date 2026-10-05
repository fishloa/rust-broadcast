# Changelog

All notable changes to `multimux-cli` will be documented in this file.

## [Unreleased]


## [0.9.0] - 2026-10-05

### Changed
- Built on `multimux` 0.11.0, which carries WHIP/WHEP, push-output, RTP-input and route/output-
  auth security fixes (some via webrtc-runtime 0.2.0) — including the
  single-publisher rule for concurrent RTMP/WHIP ingest, startup warnings for unauthenticated
  ingest routes, and stale (rather than silently re-prompted) Digest challenges; see its release
  note. No CLI changes.

- The `0.9.0` entry below says "No CLI changes", which understates it: the
  `multimux` 0.11 breaking changes alter what the CLI's `--config` / flag-built
  routes accept and serve. A config that worked on `0.8.x` may now be
  rejected, or serve differently:
  - **Route names** must be a single safe path segment (`[A-Za-z0-9._-]`, not
    `.`/`..`, at most 255 bytes), and names differing only in case collide.
  - **SRT push URLs** are validated at startup: a missing host, a `mode`
    other than `caller`, a `passphrase`, an out-of-range `latency` or an
    unbracketed IPv6 authority is an error; `streamid`/`latency` query
    parameters are now applied rather than treated as part of the address.
  - **Reconnect/timeout validation is stricter**: a zero backoff, an
    `initial_backoff_ms` above `max_backoff_ms`, a backoff over 24 h, or an
    `ingest_connect_timeout_secs`/`ingest_read_timeout_secs` outside
    `(0, 86400]` (or non-finite) is a config error; a non-finite or
    non-positive `target_duration_secs` is rejected too.
  - **Two outputs mounting the same manifest path** are rejected (multiple
    push/`custom`/`whep` outputs on one route remain valid).
  - **Smooth Streaming manifest shape changed** (per-`StreamIndex` timelines,
    real `QualityLevel@Bitrate`, no HEVC); clients keyed on the old values
    must be updated.
  - **Resources are instance-named**: init/segment/part URIs now carry a
    per-origin instance token and only those are cached `immutable`; a route
    restart renames them.
  - New output-visible behaviour: CORS preflight allows `Authorization`, and
    the global concurrency bound answers `503` + `Retry-After`.

## [0.8.0] - 2026-08-14

### Changed

- Build against `multimux` 0.10 (was 0.9), which adds the `file` input scheme —
  a local media file as a route source, identified with `container-probe`,
  paced to wall clock, with optional looping.

  Minor rather than patch **because the caret epoch moved**. A published 0.7.x
  requiring `multimux ^0.9` alongside a 0.7.y requiring `^0.10` would put two
  incompatible epochs in one compatibility bucket, so a `cargo update` within
  `0.7` could silently change which `multimux` a consumer resolves. Every
  published bucket stays epoch-pure (`docs/RELEASE-AUDIT.md` §2). The rule is
  machine-checked by `tools/check-published-dep-consistency.py`, which is what
  caught this: the bump was missed when the rest of the wave was staged.

  No CLI surface changed — same flags, same config schema. `file` routes are
  configured through `--config`, which hands routes to `multimux` unaltered.

## [0.7.0] - 2026-08-11

### Changed
- MSRV raised to **1.95.0** (issue #949). This removes the workspace's MSRV
  split: `webrtc-runtime`'s optional `media` feature needed rustc 1.88 (via
  `rcgen`), which had grown a dedicated CI job, six `--exclude` lanes and a
  guard script to contain. Adopting let-chains and `is_multiple_of` where the
  1.95 lints require them; no functional or API change.

## [0.6.0] - 2026-08-07

### Added
- `--srt-push <URL>`, `--rtmp-push <URL>`, `--rtsp-push <URL>` — push
  outputs for relaying ingested media to downstream servers (#744).

### Changed
- Requires `multimux` 0.8 (push re-egress outputs).

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
