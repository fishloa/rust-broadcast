---
status: DRAFT
---
# multimux-cli 0.11.0

_Released 2026-10-16._

**Epoch-forced major-class release.** No source change: `multimux-cli` follows `multimux` 0.13's caret epoch. The route JSON gains `allow_open_ingest` (default `false`) and WHIP/WHEP clients must follow the per-session `Location`; see the multimux note.

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

No CLI flag changed. A config that exposes an unauthenticated ingest listener on a non-loopback address now fails validation unless it sets `"allow_open_ingest": true`.

## What changed (from CHANGELOG `[Unreleased]`)

_(No `[Unreleased]` entries in this crate's CHANGELOG.)_

## Merged changes since `multimux-cli-v0.10.0`

_(no commits touching this crate)_
