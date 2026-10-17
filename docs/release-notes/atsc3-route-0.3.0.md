---
status: DRAFT
---
# atsc3-route 0.3.0

_Released 2026-10-16._

**Epoch-forced major-class release; no source migration.** `atsc3-route` has no API change of its own in this release. It ships because it builds against `broadcast-common` 10 and `rmt-flute` 0.7, and the workspace's epoch-purity rule forbids publishing a crate that still requires the old epoch next to the new one. The crate is archived (ATSC 3.0 work in this workspace is abandoned and the sibling `atsc3` crate stays unpublished); this release is a maintenance release. The README and crate-root docs were restructured to say exactly what is and is not covered, including a known repair-flow coverage gap.

## Why this version

- **Proposed version: 0.2.0 -> 0.3.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** none for consumers. Since `atsc3-route-v0.2.0` the crate gained tests (a serializer length-truncation suite and a shared fixture loader), the `serde` dependency was made `no_std`-compatible, and the docs were rewritten.
- **cargo-semver-checks vs live 0.2.0 (all features):** none flagged. It is lint-based and misses type changes of existing fields and arguments, so the CHANGELOG is the primary evidence.
- **Epoch cascade:** from `broadcast-common` (9 -> 10) and `rmt-flute` (0.6 -> 0.7), both of which move epoch in this wave.
- **Caret epoch:** 0.2 -> 0.3; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = { path = "../broadcast-common", version = "9.4",  default-features = false }
+broadcast-common = { path = "../broadcast-common", version = "10.0", default-features = false }
-rmt-flute        = { path = "../rmt-flute",        version = "0.6",  default-features = false }
+rmt-flute        = { path = "../rmt-flute",        version = "0.7",  default-features = false }
```

The optional `serde` dependency is now `default-features = false` with `features = ["derive", "alloc"]`, and the `std` feature forwards `serde?/std`, so enabling `serde` without `std` works in a `no_std` build.

## Breaking changes and migration

No source-level migration in this crate's own API. Bump the requirement to `atsc3-route = "0.3"` and make sure your dependency graph resolves a single `broadcast-common` 10 and `rmt-flute` 0.7. If your code calls `Serialize::to_bytes` on `atsc3-route` types, that default method no longer exists in `broadcast-common` 10; use `try_to_bytes`:

```rust
// before
let wire = value.to_bytes();
// after
let wire = value.try_to_bytes()?;
```

## What changed (from CHANGELOG `[Unreleased]`)

### Documentation
- README and crate-root docs restructured: archived status, what is / is not covered, standards table (A/331 Annex A, delegated RFCs), feature flags, known repair-flow coverage gap.

### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `rmt-flute` 0.6 -> 0.7). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.
