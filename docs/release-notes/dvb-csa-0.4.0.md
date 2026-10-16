---
status: DRAFT
---
# dvb-csa 0.4.0

_Released 2026-10-16._

**Epoch-forced major-class release.** Constant-time `ControlWord` equality (`subtle`) plus a `secret_debug_guard`; the release is a major only because `mpeg-ts` (a dependency) moves to 0.6.

## Why this version

- **Proposed version:** 0.3.1 -> **0.4.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** patch (constant-time ControlWord eq).
- **cargo-semver-checks vs live 0.3.1 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** epoch cascade from mpeg-ts. Siblings it depends on that move epoch in this wave: mpeg-ts.
- **Caret epoch:** 0.3 -> 0.4; every workspace dependent must move with it.

## Dependency changes

```toml
-mpeg-ts = "0.5"
+mpeg-ts = "0.6"
```

## Breaking changes and migration

No API change. `ControlWord: PartialEq` now compares via `subtle::ConstantTimeEq`.

## What changed (from CHANGELOG `[Unreleased]`)

#### Security (secrets and identity handling, #1142)

- `ControlWord` equality now uses `subtle::ConstantTimeEq` instead of a hand-written XOR fold (new `subtle` dependency); new `tests/secret_debug_guard.rs`.

## Merged changes since `dvb-csa-v0.3.1`

- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
