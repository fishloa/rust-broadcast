---
status: DRAFT
---
# mpeg-ps 0.6.0

_Released 2026-10-16._

**Breaking release.** `SystemHeader` preserves the bytes `header_length` declares after the stream-bound loop (`trailing_reserved`), instead of dropping them and shrinking `header_length` on re-serialize.

## Why this version

- **Proposed version:** 0.5.0 -> **0.6.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (SystemHeader.trailing_reserved field).
- **cargo-semver-checks vs live 0.5.0 (all features):** MAJOR (1 major, 0 minor lint failures). (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** none needed (own breaking change). Siblings it depends on that move epoch in this wave: broadcast-common, mpeg-pes.
- **Caret epoch:** 0.5 -> 0.6; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-mpeg-pes = "0.5"
+mpeg-pes = "0.6"
```

## Breaking changes and migration

```rust
// before
SystemHeader { .., reserved_bits, stream_bounds }
// after
SystemHeader { .., reserved_bits, stream_bounds, trailing_reserved: Vec::new() }
```

## What changed (from CHANGELOG `[Unreleased]`)

#### Changed
- Internal: the 27 MHz PCR constants / modular delta / `base * 300 + extension` arithmetic now come from `broadcast_common::clock33` (`PCR_HZ`, `PCR_WRAP_27MHZ`, `pcr_forward_distance`, `pcr_from_parts`, `pcr_split`) instead of a local copy; no behaviour change (#1206).
#### Changed (breaking)
- `SystemHeader` gains `trailing_reserved: Vec<u8>`: bytes `header_length` declares after the stream-bound loop are preserved and re-serialized (previously dropped, shrinking `header_length`) (audit r14-MPS-W8, #1119).

## Merged changes since `mpeg-ps-v0.5.0`

- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- clock33: shared PCR constants + modular distance; migrate 5 crates; fix ts-fix post-wrap observations (#1206) [stacked on #1196] (#1207)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Audit remediation: unsafe quarantine, panic removal, workspace lints/deps (#1156)
