---
status: DRAFT
---
# ssai-runtime 0.3.0

_Released 2026-10-16._

**Epoch-forced major-class release.** Epoch-forced release; no change of its own. Its public API does not change in a way that needs source migration; it ships because a workspace sibling it depends on moves to a new caret epoch (see below), and the workspace's epoch-purity rule forbids publishing a crate that still requires the old epoch next to the new one.

## Why this version

- **Proposed version:** 0.2.1 -> **0.3.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** none (no change).
- **cargo-semver-checks vs live 0.2.1 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** no change; epoch cascade from broadcast-common. Siblings it depends on that move epoch in this wave: broadcast-common, broadcast-hls, mp4-emsg, scte35-splice.
- **Caret epoch:** 0.2 -> 0.3; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-broadcast-hls = "0.3"
+broadcast-hls = "0.4"
-mp4-emsg = "0.4" # dev-dependency
+mp4-emsg = "0.5" # dev-dependency
-scte35-splice = "3.0" # dev-dependency
+scte35-splice = "4.0" # dev-dependency
```

## Breaking changes and migration

No source-level migration is required beyond bumping the version requirement and, if the crate exposes `broadcast-common` types in its API, moving to `broadcast-common` 10 in the same dependency graph.

## What changed (from CHANGELOG `[Unreleased]`)

_(No `[Unreleased]` entries in this crate's CHANGELOG.)_

## Merged changes since `ssai-runtime-v0.2.1`

- clock33: shared PCR constants + modular distance; migrate 5 crates; fix ts-fix post-wrap observations (#1206) [stacked on #1196] (#1207)
- Audit sweep of the nine small crates: fit_mask serializers, hostility + guard tests (#1202) [stacked on #1199] (#1203)
