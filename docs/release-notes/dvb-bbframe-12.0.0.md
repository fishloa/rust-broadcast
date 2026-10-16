---
status: DRAFT
---
# dvb-bbframe 12.0.0

_Released 2026-10-16._

**Breaking release.** Lockstep major. `Bbheader::serialize()` returns `Result<[u8; BBHEADER_LEN], Error>` instead of a bare array that panicked on a hand-built header violating a wire constraint (e.g. `dfl > DFL_MAX_BITS`), and serializes straight into a stack array.

## Why this version

- **Proposed version:** 11.1.0 -> **12.0.0** (major-class: semver major).
- **Own API/behaviour change class:** major (Bbheader::serialize returns Result).
- **cargo-semver-checks vs live 11.1.0 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** none needed (own breaking change). Siblings it depends on that move epoch in this wave: broadcast-common.
- **Caret epoch:** 11 -> 12; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
```

## Breaking changes and migration

```rust
// before
let hdr: [u8; BBHEADER_LEN] = bbheader.serialize();
// after
let hdr: [u8; BBHEADER_LEN] = bbheader.serialize()?;
```

## What changed (from CHANGELOG `[Unreleased]`)

#### Changed
- Internal: adapted to `broadcast-common`'s removal of the panicking `Serialize::to_bytes` (tests, examples and docs now use `try_to_bytes`); requires the new `broadcast-common` major (umbrella #1143).
- **BREAKING**: `Bbheader::serialize()` now returns
  `Result<[u8; BBHEADER_LEN], Error>` (was a bare array that panicked, via
  `to_bytes()`, on a hand-built header violating a wire constraint such as
  `dfl > DFL_MAX_BITS`). It also serializes straight into a stack array
  (r11-W-BB-2 / r11-O-BB-2, #1094).

## Merged changes since `v11.1.0`

- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
