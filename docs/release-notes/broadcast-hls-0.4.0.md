---
status: DRAFT
---
# broadcast-hls 0.4.0

_Released 2026-10-16._

**Epoch-forced major-class release; no source migration.** `broadcast-hls` has one behavioural fix of its own (`computed_version` no longer over-counts the open segment's `EXT-X-MAP`), but no public API change. It ships as 0.4.0 because it builds against `broadcast-common` 10, and the workspace's epoch-purity rule forbids publishing a crate that still requires the old epoch next to the new one. The README and crate-root docs were also restructured to state what the crate does and does not do.

## Why this version

- **Proposed version: 0.3.1 -> 0.4.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** patch. The `computed_version` fix changes the `EXT-X-VERSION` value rendered in one narrow case (low-latency off, open segment not rendered); it is a correction, not an API change.
- **cargo-semver-checks vs live 0.3.1 (all features):** none flagged. It is lint-based and misses type changes of existing fields and arguments, so the CHANGELOG is the primary evidence.
- **Epoch cascade:** from `broadcast-common` (9 -> 10).
- **Caret epoch:** 0.3 -> 0.4; every workspace dependent (`transmux`, `hls-runtime`, `multimux`, `ssai-runtime`, `media-doctor` and others) must move with it.

## Dependency changes

```toml
-broadcast-common = { path = "../broadcast-common", version = "9.4",  default-features = false }
+broadcast-common = { path = "../broadcast-common", version = "10.0", default-features = false }
```

## Breaking changes and migration

No source-level migration in this crate's own API. Bump the requirement to `broadcast-hls = "0.4"` and make sure your dependency graph resolves a single `broadcast-common` 10 (the `serde` feature forwards `broadcast-common/serde`, so the two must agree). If your code calls `Serialize::to_bytes` on a `broadcast-common` `Serialize` type, that default method is gone in 10:

```rust
// before
let wire = value.to_bytes();
// after
let wire = value.try_to_bytes()?;
```

## What changed (from CHANGELOG `[Unreleased]`)

### Documentation
- README restructured: what the crate does and does not do, a standards-supported table (edition, scope, status), feature-flag table, a doctested quick start and related crates; crate-root docs now render the README (or are kept in step with it).

### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

### Fixed
- `computed_version` no longer counts the open segment's `EXT-X-MAP` (or its `REQ-` attributes) when low-latency is off and the segment is not rendered (audit BH-O3, #1111).
