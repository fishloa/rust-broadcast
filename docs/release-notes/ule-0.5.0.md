---
status: DRAFT
---
# ule 0.5.0

_Released 2026-10-16._

**Breaking release (own API change plus the `broadcast-common` 10 epoch).** The never-constructed `Error::TsMapping` variant is removed, the TS receiver hands a completed SNDU out by move (`split_off`) instead of copying it and its tail, and the crate now builds against `broadcast-common` 10. The README and crate-root docs are restructured (what is and is not covered, standards table for RFC 4326 and RFC 5163, feature flags, design notes).

## Why this version

- **Proposed version:** 0.4.1 -> **0.5.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (Error::TsMapping removed).
- **cargo-semver-checks vs live 0.4.1 (all features):** MAJOR (1 major, 0 minor lint failures). (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** none needed (own breaking change). Siblings it depends on that move epoch in this wave: broadcast-common.
- **Caret epoch:** 0.4 -> 0.5; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
```

## Breaking changes and migration

`Error::TsMapping` was never returned by any code path, so only exhaustive matches on `ule::Error` are affected:

```rust
// before
match err { Error::TsMapping => handle_ts_mapping(), other => return Err(other) }
// after: delete the arm (the variant never occurred)
match err { other => return Err(other) }
```

Also move every `broadcast-common` requirement in your tree to `10` so one copy resolves; `Serialize::to_bytes` was removed there (it could panic), so use `try_to_bytes()?`. The TS receiver change is behavioural only (fewer copies, same bytes out).

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- README and crate-root docs restructured: what is / is not covered, standards table (RFC 4326, RFC 5163), feature flags, design notes.

#### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

#### Removed
- **BREAKING:** the never-constructed `Error::TsMapping` variant (audit r14-ULE-O1, #1120).

#### Changed
- The TS receiver hands a completed SNDU out by move (`split_off`) instead of copying it and its tail (audit r14-ULE-O1, #1120).

## Merged changes since `ule-v0.4.1`

- docs: cluster C crate READMEs + crate-root docs (standards tables, feature flags, scope, verified editions) (#1284)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Audit remediation: unsafe quarantine, panic removal, workspace lints/deps (#1156)
