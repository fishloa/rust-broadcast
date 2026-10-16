---
status: DRAFT
---
# mp4-emsg 0.5.0

_Released 2026-10-16._

**Breaking release.** `EmsgBox` gains a typed `flags: u32` (the FullBox 24-bit flags, mandated 0), so a non-conformant box round-trips byte-exactly; `is_scte35` no longer matches `urn:scte:scte35x...`.

## Why this version

- **Proposed version:** 0.4.1 -> **0.5.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (EmsgBox.flags field, parse_with_flags removed).
- **cargo-semver-checks vs live 0.4.1 (all features):** MAJOR (2 major, 0 minor lint failures). (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** none needed (own breaking change). Siblings it depends on that move epoch in this wave: broadcast-common.
- **Caret epoch:** 0.4 -> 0.5; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
```

## Breaking changes and migration

```rust
// before (0.4.1): the *_with_flags pair existed to round-trip a non-zero flags field
EmsgBox::parse_with_flags(..)      EmsgBox::serialize_into_with_flags(..)
// after: plain parse keeps the flags
let b = EmsgBox::parse(bytes)?;                 // b.flags
EmsgBox { flags: EMSG_FLAGS, .. }               // migrate struct literals
```

`parse_with_flags` / `serialize_into_with_flags` (added in 0.4.1) are removed.

## What changed (from CHANGELOG `[Unreleased]`)

#### Changed (BREAKING)
- `EmsgBox` gains a typed `flags: u32` field (the `FullBox` 24-bit flags,
  mandated 0, `EMSG_FLAGS`): plain `parse` now keeps it and `serialize_into`
  writes it back, so a non-conformant box round-trips byte-exactly (#1104 W2).
  The `parse_with_flags` / `serialize_into_with_flags` pair added in 0.4.1 is
  removed (superseded by the field). Migrate struct literals with
  `flags: EMSG_FLAGS`.

#### Fixed
- `EmsgBox::is_scte35` matches the prefix with its colon
  (`urn:scte:scte35:`), so `urn:scte:scte35x...` is no longer classed as
  SCTE 35 (#1104 O1).

## Merged changes since `mp4-emsg-v0.4.1`

- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Audit remediation: unsafe quarantine, panic removal, workspace lints/deps (#1156)
