# broadcast-loudness 0.5.0

_Released 2026-10-17._

**Breaking release (its own break, plus the `broadcast-common` 10 epoch).** The never-constructed `Error::ChannelOutOfRange` variant is removed, which breaks any exhaustive `match` on `Error`. The release also adds the Chinese loudness target profiles (GY/T 282-2014, GY/T 377-2023 quiet and noisy) with GY/T 379-2023 channel-monitoring helpers, and a 30-minute bounded-memory test that pins the streaming design. README and crate-root docs were rewritten, and the Tech 3341 case coverage is now stated as what the test suite actually exercises.

## Why this version

- **Proposed version: 0.4.0 -> 0.5.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major. `Error::ChannelOutOfRange` is removed; the `profile` module is additive.
- **cargo-semver-checks vs live 0.4.0 (all features):** MAJOR (2 major, 0 minor lint failures), consistent with the CHANGELOG BREAKING entry.
- **Epoch cascade:** also from `broadcast-common` (9 -> 10).
- **Caret epoch:** 0.4 -> 0.5; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = { path = "../broadcast-common", version = "9.4",  default-features = false }
+broadcast-common = { path = "../broadcast-common", version = "10.0", default-features = false }
```

Dev-dependencies gained the path-only `test-alloc` crate (not published, used by the new bounded-memory test).

## Breaking changes and migration

`Error::ChannelOutOfRange` was declared but never constructed (r14-LOUD-O1, #1124), so no code path could have produced it. Delete any match arm that names it:

```rust
// before
match err {
    Error::ChannelOutOfRange { .. } => { /* unreachable in practice */ }
    other => return Err(other),
}
// after: the arm is simply deleted
```

If you depend on `broadcast-common` types directly, resolve a single `broadcast-common` 10 in the same graph.

## What changed (from CHANGELOG `[Unreleased]`)

### Documentation
- README and new crate-root docs rewritten to the cluster standard: what it does / does not, standards table (BS.1770-5, R 128 v5, Tech 3341/3342 v4, GY/T 282/377/379), feature table, compiling quick start; Tech 3341 case coverage corrected to what the test suite exercises.

### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

### Added (#1236 - Chinese loudness profiles)
- `profile` module: `LoudnessProfile` target profiles `GY_T_282_2014` (-24 LKFS +-2 LU, <= -2 dBTP), `GY_T_377_2023_QUIET` (-24 LKFS +-2 LU, <= -1 dBTP) and `GY_T_377_2023_NOISY` (-15 LKFS +-2 LU, <= -1 dBTP) with `check`/`check_meters` -> `ProfileReport`; GY/T 379-2023 channel-monitoring helpers (`MONITOR_OVER_LIMIT_LKFS` -22, `over_limit_ratio` = M10, `consecutive_over_limit_ratio` = S3, `classify_ratio`/`classify_units` with the Table 1 20%/40% verdicts `MonitorVerdict`, `MonitorUnit`). Unit tests pin every constant to the standard text and the inclusive tolerance/limit edges.
### Removed
- **BREAKING:** the never-constructed `Error::ChannelOutOfRange` variant (r14-LOUD-O1, #1124).
### Added
- `tests/bounded_memory.rs`: 30 minutes of audio through both meters under a
  16 MiB live-allocation cap, proving the streaming design of #1072.
