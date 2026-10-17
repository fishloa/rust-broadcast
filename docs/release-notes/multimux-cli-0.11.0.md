---
status: DRAFT
---
# multimux-cli 0.11.0

_Released 2026-10-16._

**Epoch-forced major-class release.** No source change: `multimux-cli` follows `multimux` 0.13's caret epoch. The route JSON gains `allow_open_ingest` (default `false`) and WHIP/WHEP clients must follow the per-session `Location`; see the multimux note. Because the binary is a thin wrapper over the library it also picks up, with no flag changes, the multimux 0.13 ingest fixes: no lost first/last access units on TS, FLV and RTP inputs, stable `ProgramId`s on multi-program transport streams, and the new `multimux_ts_backlog_dropped_total` metric.

## Why this version

- **Proposed version:** 0.10.0 -> **0.11.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** none (no change).
- **cargo-semver-checks vs live 0.10.0 (all features):** tool error. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** no change; epoch cascade from multimux. Siblings it depends on that move epoch in this wave: multimux.
- **Caret epoch:** 0.10 -> 0.11; every workspace dependent must move with it.

## Dependency changes

```toml
-multimux = "0.12"
+multimux = "0.13"
```

## Breaking changes and migration

No CLI flag changed, and the single-route quick start (`--rtsp`/`--name`, `--outputs`/`--dash`) is untouched. A config that exposes an unauthenticated ingest listener on a non-loopback address now fails validation unless it sets `"allow_open_ingest": true` in the `--config` JSON:

```json
{ "allow_open_ingest": true }
```

A WHIP/WHEP client that hard-coded `/whip/session` must follow the `Location` header (`/whip/session/<id>`), and a trickle `PATCH` must send `If-Match` (otherwise `428`). Pull credentials are sent only to the origin of the configured source URL. Details and more in the multimux 0.13.0 note.

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- README restructured: what the crate does and does not do, a standards-supported table (edition, scope, status), feature-flag table, a doctested quick start and related crates; crate-root docs now render the README (or are kept in step with it).

#### Changed (BREAKING: epoch — requires multimux 0.13)
- Builds against new caret epochs of workspace siblings (`multimux` 0.12 -> 0.13). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff.

## Merged changes since `multimux-cli-v0.10.0`

- docs: clear per-crate documentation for cluster B (16 crates) (#1291)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
