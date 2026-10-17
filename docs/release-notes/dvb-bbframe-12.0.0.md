# dvb-bbframe 12.0.0

_Released 2026-10-17._

**Breaking release (its own break, lockstep major).** `Bbheader::serialize()` now returns `Result<[u8; BBHEADER_LEN], Error>` instead of a bare array. The old form panicked (via `to_bytes()`) on a hand-built header that violates a wire constraint such as `dfl > DFL_MAX_BITS`; the new form reports `Error::DflOutOfRange` or `Error::InconsistentBbheaderMode` and serializes straight into a stack array. The crate also moves to `broadcast-common` 10 and the README and crate-root docs were rewritten to the workspace standard. `dvb-bbframe` is one of the five lockstep crates, so it takes the shared 12.0.0 version.

## Why this version

- **Proposed version: 11.1.0 -> 12.0.0** (major-class: semver major; lockstep with `dvb-si`, `dvb-t2mi`, `dvb-conformance`, `dvb-tools`).
- **Own API/behaviour change class:** major. `Bbheader::serialize` changes its return type.
- **cargo-semver-checks vs live 11.1.0 (all features):** none flagged. It does not see a changed return type, so the CHANGELOG BREAKING entry is the primary evidence.
- **Epoch cascade:** also from `broadcast-common` (9 -> 10).
- **Caret epoch:** 11 -> 12; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = { path = "../broadcast-common", version = "9.4",  default-features = false }
+broadcast-common = { path = "../broadcast-common", version = "10.0", default-features = false }
```

The optional `serde` dependency is now `default-features = false` with `features = ["derive", "alloc"]`, and `std` forwards `serde?/std`.

## Breaking changes and migration

```rust
// before: returned a bare array and panicked on an invalid hand-built header
let hdr: [u8; BBHEADER_LEN] = bbheader.serialize();
// after: propagate the error
let hdr: [u8; BBHEADER_LEN] = bbheader.serialize()?;
```

The error is `Error::DflOutOfRange` or `Error::InconsistentBbheaderMode`, which is what a hand-built header violating a wire constraint now reports instead of panicking. The `Serialize` trait path (`serialize_into`, `try_to_bytes`) is unchanged apart from `broadcast-common` 10 removing the panicking `to_bytes` default; use `try_to_bytes()` where you previously called `to_bytes()` on the trait.

## What changed (from CHANGELOG `[Unreleased]`)

### Documentation
- README rewritten to the workspace crate-README standard: what the crate does and does not do, a standards table (edition, scope, status), feature flags, a doctested quick start, the implemented API surface, design notes and related crates. Where the crate has a library target the crate-root docs are now `include_str!("../README.md")`, so docs.rs and crates.io show the same text.

### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

### Changed
- **BREAKING**: `Bbheader::serialize()` now returns
  `Result<[u8; BBHEADER_LEN], Error>` (was a bare array that panicked, via
  `to_bytes()`, on a hand-built header violating a wire constraint such as
  `dfl > DFL_MAX_BITS`). It also serializes straight into a stack array
  (r11-W-BB-2 / r11-O-BB-2, #1094).
