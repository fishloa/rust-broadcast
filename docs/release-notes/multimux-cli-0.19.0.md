# multimux-cli 0.19.0

_Released 2026-11-22._

Minor for a 0.x crate, with no functional change: the binary is republished because it builds against `multimux` 0.21, a caret-epoch change that is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2). `--config <FILE>` and the single-route quick start (`--rtsp`/`--name`, `--outputs`/`--dash`) are unchanged, and so is the JSON route schema.

## Breaking changes

- Builds against `multimux` 0.21 (and, through it, `broadcast-common` 14 and `dvr-runtime` 0.1). A consumer that embeds the binary needs no change; a consumer of the config schema sees only `multimux`'s own DVR archive-format change (an archive written by 0.20 or earlier is not readable, so start on an empty directory).

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #4009 (the empty `[Unreleased]` now names the epoch), #4026 (the crate is kept in the release pairs file even though `release_plan.py plan` reports it "live").

---

Published from tag `multimux-cli-v0.19.0`.
