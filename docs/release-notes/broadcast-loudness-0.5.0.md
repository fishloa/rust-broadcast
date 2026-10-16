---
status: DRAFT
---
# broadcast-loudness 0.5.0

_Released 2026-10-16._

**Breaking release.** Adds the Chinese loudness profiles (GY/T 282-2014, GY/T 377-2023 quiet/noisy) and GY/T 379-2023 channel-monitoring helpers; the never-constructed `Error::ChannelOutOfRange` is removed; a 30-minute bounded-memory test pins the streaming design.

## Why this version

- **Proposed version:** 0.4.0 -> **0.5.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (Error::ChannelOutOfRange removed (semver-checks major)).
- **cargo-semver-checks vs live 0.4.0 (all features):** MAJOR (2 major, 0 minor lint failures). (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** none needed (own breaking change). Siblings it depends on that move epoch in this wave: broadcast-common.
- **Caret epoch:** 0.4 -> 0.5; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
```

## Breaking changes and migration

```rust
// before: match e { Error::ChannelOutOfRange { .. } => .., .. }
// after: delete the arm (never constructed)
```

## What changed (from CHANGELOG `[Unreleased]`)

#### Added (#1236 - Chinese loudness profiles)
- `profile` module: `LoudnessProfile` target profiles `GY_T_282_2014` (-24 LKFS +-2 LU, <= -2 dBTP), `GY_T_377_2023_QUIET` (-24 LKFS +-2 LU, <= -1 dBTP) and `GY_T_377_2023_NOISY` (-15 LKFS +-2 LU, <= -1 dBTP) with `check`/`check_meters` -> `ProfileReport`; GY/T 379-2023 channel-monitoring helpers (`MONITOR_OVER_LIMIT_LKFS` -22, `over_limit_ratio` = M10, `consecutive_over_limit_ratio` = S3, `classify_ratio`/`classify_units` with the Table 1 20%/40% verdicts `MonitorVerdict`, `MonitorUnit`). Unit tests pin every constant to the standard text and the inclusive tolerance/limit edges.
#### Removed
- **BREAKING:** the never-constructed `Error::ChannelOutOfRange` variant (r14-LOUD-O1, #1124).
#### Added
- `tests/bounded_memory.rs`: 30 minutes of audio through both meters under a
  16 MiB live-allocation cap, proving the streaming design of #1072.

## Merged changes since `broadcast-loudness-v0.4.0`

- feat(broadcast-loudness): Chinese loudness profiles GY/T 282/377 and GY/T 379 monitoring helpers (#1236) (#1241)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Audit remediation: unsafe quarantine, panic removal, workspace lints/deps (#1156)
