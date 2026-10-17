---
status: DRAFT
---
# rtmp-runtime 0.8.0

_Released 2026-10-16._

**Epoch-forced major-class release.** Stream keys are compared in constant time and never printed by `Debug`; an RTMP message length over 24 bits is rejected (`RtmpError::FieldOverflow`) instead of dropping the high byte.

## Why this version

- **Proposed version:** 0.7.2 -> **0.8.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** minor (RtmpError::FieldOverflow (non_exhaustive, additive) + hardening).
- **cargo-semver-checks vs live 0.7.2 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** epoch cascade from broadcast-common. Siblings it depends on that move epoch in this wave: broadcast-common, transmux.
- **Caret epoch:** 0.7 -> 0.8; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-transmux = "0.26" # dev-dependency
+transmux = "0.27" # dev-dependency
```

## Breaking changes and migration

`RtmpError::FieldOverflow` is added (`#[non_exhaustive]`). `ServerConfig::expected_stream_key = None` (the default) still admits any publisher: documented, set a key on untrusted networks. `UserControl::to_message` / `ProtocolControl::to_message` keep their documented panic via an explicit `expect` (use `try_to_message` for untrusted values). The major is the `broadcast-common` epoch cascade.

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- README restructured: what the crate does and does not do, a standards-supported table (edition, scope, status), feature-flag table, a doctested quick start and related crates; crate-root docs now render the README (or are kept in step with it).

#### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `transmux` 0.26 -> 0.27). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

#### Changed
- `UserControl::to_message` / `ProtocolControl::to_message` keep their documented panic via an explicit `expect` message (use `try_to_message` for untrusted values).

#### Security (secrets and identity handling, #1142)

- `Debug` of `ServerConfig`, `ClientConfig`, `RtmpTarget`, `ServerEvent` (and therefore `ServerSession`) no longer prints the stream key; `Media` prints its length, not its bytes.
- The stream-key comparison uses `subtle::ConstantTimeEq` (new `subtle` dependency) instead of a hand-written fold.
- Documented: `ServerConfig::expected_stream_key = None` (the default) admits any publisher; deployments reachable from untrusted networks must set a key. New `tests/secret_hygiene.rs`, `tests/secret_debug_guard.rs`.

#### Fixed

- **RTMP `message length` over 24 bits is rejected (#1129).** `MessageHeader::{Type0,Type1}` wrote the caller-supplied `u32` with the high byte silently dropped; `serialize_into` now returns `RtmpError::FieldOverflow`.

#### Fixed
- `ServerSession::handle_data` no longer copies every post-handshake read into a scratch `Vec` before the chunk assembler buffers it (one copy instead of two; measured 246 976 -> 186 976 bytes for a 60 000-byte read) (#1085/RTMP-O1). `test-alloc` added as a dev-dependency for the allocation-count test.

## Merged changes since `rtmp-runtime-v0.7.2`

- docs: clear per-crate documentation for cluster B (16 crates) (#1291)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
