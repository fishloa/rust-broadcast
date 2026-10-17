---
status: DRAFT
---
# dvb-csa 0.4.0

_Released 2026-10-16._

**Epoch-forced major-class release; no source migration.** `ControlWord` equality is now constant-time (`subtle::ConstantTimeEq` instead of a hand-written XOR fold, #1142), and a new `tests/secret_debug_guard.rs` guards against secret-bearing `Debug` output. Neither changes the public API. The release is a major only because `mpeg-ts`, a dependency whose `ScramblingControl` type appears in this crate's public `Error` variants, moves to 0.6 and the workspace's epoch-purity rule forbids publishing a crate that still requires the old epoch next to the new one. README and crate-root docs were rewritten to the workspace standard.

## Why this version

- **Proposed version: 0.3.1 -> 0.4.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** patch. `ControlWord: PartialEq` keeps its semantics and signature; only the comparison's timing behaviour changes.
- **cargo-semver-checks vs live 0.3.1 (all features):** none flagged.
- **Epoch cascade:** from `mpeg-ts` (0.5 -> 0.6).
- **Caret epoch:** 0.3 -> 0.4; every workspace dependent must move with it.

## Dependency changes

```toml
-mpeg-ts = { path = "../mpeg-ts", version = "0.5", default-features = false }
+mpeg-ts = { path = "../mpeg-ts", version = "0.6", default-features = false }
+subtle   = { version = "2", default-features = false }     # new: constant-time control-word comparison
```

`subtle` is a new third-party dependency (`no_std`, no features).

## Breaking changes and migration

No source change in this crate's own API. Bump the requirement to `dvb-csa = "0.4"` and make sure your dependency graph resolves a single `mpeg-ts` 0.6, because `Error`'s scrambling-control variants carry `mpeg_ts::ts::ScramblingControl`. `ControlWord == ControlWord` compiles and behaves as before, now in constant time.

## What changed (from CHANGELOG `[Unreleased]`)

### Documentation
- README and crate-root docs rewritten to the workspace documentation standard: what the crate does and does not do, a standards-supported table (edition, scope, status), feature flags, a doctested quick start, and related crates.

### Changed (BREAKING: epoch — requires mpeg-ts 0.6)
- Builds against new caret epochs of workspace siblings (`mpeg-ts` 0.5 -> 0.6). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff.

### Security (secrets and identity handling, #1142)

- `ControlWord` equality now uses `subtle::ConstantTimeEq` instead of a hand-written XOR fold (new `subtle` dependency); new `tests/secret_debug_guard.rs`.
