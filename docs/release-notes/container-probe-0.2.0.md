---
status: DRAFT
---
# container-probe 0.2.0

_Released 2026-10-16._

**Epoch-forced major-class release.** Epoch-forced release. Own change since `container-probe-v0.1.1`: prober verdict fix + allocation bound. Its public API does not change in a way that needs source migration; it ships because a workspace sibling it depends on moves to a new caret epoch (see below), and the workspace's epoch-purity rule forbids publishing a crate that still requires the old epoch next to the new one.

## Why this version

- **Proposed version:** 0.1.1 -> **0.2.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** patch (prober verdict fix + allocation bound).
- **cargo-semver-checks vs live 0.1.1 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** epoch cascade from broadcast-common. Siblings it depends on that move epoch in this wave: broadcast-common, mpeg-ps, mpeg-ts, st377-1.
- **Caret epoch:** 0.1 -> 0.2; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-mpeg-ps = "0.5" # dev-dependency
+mpeg-ps = "0.6" # dev-dependency
-mpeg-ts = "0.5" # dev-dependency
+mpeg-ts = "0.6" # dev-dependency
-st377-1 = "0.4" # dev-dependency
+st377-1 = "0.5" # dev-dependency
```

## Breaking changes and migration

No source-level migration is required beyond bumping the version requirement and, if the crate exposes `broadcast-common` types in its API, moving to `broadcast-common` 10 in the same dependency graph.

## What changed (from CHANGELOG `[Unreleased]`)

#### Changed
- A prefix too short for a literal magic that already contradicts it (`XY` for `OggS`, `FLV`, ASF GUID, MXF key, PS start code, EBML magic, `RIFF`..`WAVE`) is now `None` ("stop") for that prober instead of `Insufficient` ("read more"); a genuine truncated prefix of the same magic stays `Insufficient`. Consequence: eight zero bytes now ask for the TS lattice's structural need (1504) instead of the geometric floor (13) (audit r01-O6, #1076).
- The ADTS and MP3 chain walkers are one generic `chain::longest_chain` (r01-O5); the crate docs' allocation claim is now accurate and pinned by `tests/alloc_bound.rs` (r01-O7).

## Merged changes since `container-probe-v0.1.1`

- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Audit remediation: unsafe quarantine, panic removal, workspace lints/deps (#1156)
