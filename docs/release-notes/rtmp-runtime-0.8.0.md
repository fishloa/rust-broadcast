# rtmp-runtime 0.8.0

_Released 2026-10-16._

**Epoch-forced major-class release with hardening of its own.** `rtmp-runtime` follows the `broadcast-common` 9 -> 10 caret epoch (and `transmux` 0.26 -> 0.27 as a dev-dependency). Its own changes: stream keys are compared in constant time (`subtle::ConstantTimeEq`) and never printed by `Debug` (`ServerConfig`, `ClientConfig`, `RtmpTarget`, `ServerEvent`, `ServerSession`; `Media` prints its length, not its bytes); an RTMP message length over 24 bits is rejected with the new `RtmpError::FieldOverflow` instead of silently dropping the high byte (#1129); and `ServerSession::handle_data` copies each post-handshake read once instead of twice (60,000-byte read: 246,976 -> 186,976 bytes allocated).

## Why this version

- **Proposed version:** 0.7.2 -> **0.8.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** minor (additive `#[non_exhaustive]` `RtmpError::FieldOverflow`, a serializer that now errors instead of truncating, secret-hygiene hardening, a new `subtle` dependency).
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

### 1. Over-long message lengths are an error, not a truncation

`MessageHeader::{Type0,Type1}::serialize_into` wrote the caller-supplied `u32` message length with its high byte dropped. It now returns `RtmpError::FieldOverflow` for a length that does not fit 24 bits (a 16 MiB payload or larger). `RtmpError` is `#[non_exhaustive]`, so a wildcard arm already covers the new variant:

```rust
match err {
    RtmpError::FieldOverflow(_) => reject_oversized_message(),
    _ => other(err),
}
```

### 2. Stream keys

`Debug` output of `ServerConfig`, `ClientConfig`, `RtmpTarget`, `ServerEvent` and `ServerSession` no longer contains the stream key, so log lines that relied on seeing it will not. Comparison is constant-time. Unchanged and documented: `ServerConfig::expected_stream_key = None` (the default) admits any publisher; set a key on any network you do not control. `UserControl::to_message` / `ProtocolControl::to_message` keep their documented panic (now via an explicit `expect` message); use `try_to_message` for untrusted values.

### 3. `broadcast-common` 10

`broadcast-common` 10 removed `Serialize::to_bytes()` (the default method that panicked when `serialize_into` failed). Code that serialises any `rtmp-runtime` type through it must switch to the fallible form:

```rust
// before
let bytes = value.to_bytes();
// after
let bytes = value.try_to_bytes()?;   // Result<Vec<u8>, _>
```

Your dependency graph must resolve a single `broadcast-common` 10 (a second copy on 9.x is a different `Serialize` trait and the types will not satisfy it).

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
