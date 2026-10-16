---
status: DRAFT
---
# broadcast-hls 0.4.0

_Released 2026-10-16._

**Epoch-forced major-class release.** Epoch-forced release. Own change since `broadcast-hls-v0.3.1`: computed_version fix. Its public API does not change in a way that needs source migration; it ships because a workspace sibling it depends on moves to a new caret epoch (see below), and the workspace's epoch-purity rule forbids publishing a crate that still requires the old epoch next to the new one.

## Why this version

- **Proposed version:** 0.3.1 -> **0.4.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** patch (computed_version fix).
- **cargo-semver-checks vs live 0.3.1 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** epoch cascade from broadcast-common. Siblings it depends on that move epoch in this wave: broadcast-common.
- **Caret epoch:** 0.3 -> 0.4; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
```

## Breaking changes and migration

No source-level migration is required beyond bumping the version requirement and, if the crate exposes `broadcast-common` types in its API, moving to `broadcast-common` 10 in the same dependency graph.

## What changed (from CHANGELOG `[Unreleased]`)

#### Fixed
- `computed_version` no longer counts the open segment's `EXT-X-MAP` (or its `REQ-` attributes) when low-latency is off and the segment is not rendered (audit BH-O3, #1111).

## Merged changes since `broadcast-hls-v0.3.1`

- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
