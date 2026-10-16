---
status: DRAFT
---
# dvb-mabr 0.3.0

_Released 2026-10-16._

**Epoch-forced major-class release.** Epoch-forced release. Own change since `dvb-mabr-v0.2.0`: src/manifest change, no CHANGELOG entry (defect). Its public API does not change in a way that needs source migration; it ships because a workspace sibling it depends on moves to a new caret epoch (see below), and the workspace's epoch-purity rule forbids publishing a crate that still requires the old epoch next to the new one.

## Why this version

- **Proposed version:** 0.2.0 -> **0.3.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** patch (src/manifest change, no CHANGELOG entry (defect)).
- **cargo-semver-checks vs live 0.2.0 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** epoch cascade from broadcast-common. Siblings it depends on that move epoch in this wave: broadcast-common.
- **Caret epoch:** 0.2 -> 0.3; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
```

## Breaking changes and migration

No source-level migration is required beyond bumping the version requirement and, if the crate exposes `broadcast-common` types in its API, moving to `broadcast-common` 10 in the same dependency graph.

## What changed (from CHANGELOG `[Unreleased]`)

#### Changed
- Internal: optional attributes are read with `StartTag::attr_owned` instead of building a `require_attr` error and discarding it with `.ok()` (no behaviour change; the parsed values and every error path are identical). Manifest: `quick-xml` and the lint table now come from the workspace (`[lints] workspace = true`), `test-fixtures` is a dev-dependency.
- Requires the new `broadcast-common` major (epoch release; no API change).

## Merged changes since `dvb-mabr-v0.2.0`

- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Audit remediation: unsafe quarantine, panic removal, workspace lints/deps (#1156)
